use std::collections::HashMap;
use std::ffi::OsString;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, MutexGuard,
};
use std::thread::JoinHandle;

use j3files::app::{SearchCancellation, SearchOutcome, SearchProgressReporter, SearchRequest};
use j3files::domain::{
    DisplayOptions, DropOperation, ExplorerError, ExplorerResult, FileItem, NavigationLocation,
    SearchProgress, SearchRunId, SortState, TabId,
};
use j3files::platform::{
    win32_ui as ui, DirectoryChangeBatch, DirectoryChangeCancellation, SynchronousIoCancellation,
};

use super::{
    UndoFileOperation, FILE_WATCH_REFRESH_DEBOUNCE_MS, ID_FILE_WATCH_REFRESH_TIMER,
    ID_SEARCH_COMPLETION_TIMER, MESSAGE_FILE_OPERATION_COMPLETE, MESSAGE_FILE_WATCH_CHANGED,
    MESSAGE_LISTING_COMPLETE, MESSAGE_SEARCH_COMPLETE, MESSAGE_SEARCH_PROGRESS,
    SEARCH_COMPLETION_POLL_MS,
};

const MAX_RETIRED_LISTING_WORKERS: usize = 1;
const MAX_RETIRED_FILE_WATCH_WORKERS: usize = 1;

#[derive(Debug, Clone)]
pub(super) struct SharedSearchCancellation {
    pub(super) requested: Arc<AtomicBool>,
}

impl SearchCancellation for SharedSearchCancellation {
    fn is_cancel_requested(&self) -> bool {
        self.requested.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Clone)]
pub(super) struct UiSearchProgressReporter {
    pub(super) hwnd_value: isize,
    pub(super) tab_id: TabId,
    pub(super) run_id: SearchRunId,
    pub(super) shutdown_requested: Arc<AtomicBool>,
    pub(super) worker_messages: WorkerMessageStore,
}

impl SearchProgressReporter for UiSearchProgressReporter {
    fn report(&self, progress: SearchProgress) {
        if self.shutdown_requested.load(Ordering::Relaxed) {
            return;
        }

        let message = SearchProgressMessage {
            tab_id: self.tab_id,
            run_id: self.run_id,
            progress,
        };
        self.worker_messages
            .post_search_progress(self.hwnd_value, message);
    }
}

#[derive(Debug)]
pub(super) struct ActiveSearchWorker {
    pub(super) tab_id: TabId,
    pub(super) run_id: SearchRunId,
    pub(super) cancel_requested: Arc<AtomicBool>,
    pub(super) io_cancellation: Arc<SynchronousIoCancellation>,
    pub(super) handle: Option<JoinHandle<()>>,
}

impl ActiveSearchWorker {
    pub(super) fn request_cancel(&self) {
        self.cancel_requested.store(true, Ordering::Relaxed);
        if let Err(error) = self.io_cancellation.request_cancel() {
            eprintln!(
                "search worker for tab {:?}, run {:?} cancellation failed: {error}",
                self.tab_id, self.run_id
            );
        }
    }

    pub(super) fn is_cancel_requested(&self) -> bool {
        self.cancel_requested.load(Ordering::Relaxed)
    }

    pub(super) fn is_finished(&self) -> bool {
        self.handle
            .as_ref()
            .map(|handle| handle.is_finished())
            .unwrap_or(true)
    }
}

#[derive(Debug)]
pub(super) struct PendingSearchWorker {
    pub(super) request: SearchRequest,
    pub(super) cancel_requested: Arc<AtomicBool>,
}

impl PendingSearchWorker {
    pub(super) fn tab_id(&self) -> TabId {
        self.request.tab_id
    }

    pub(super) fn run_id(&self) -> SearchRunId {
        self.request.run_id
    }

    pub(super) fn request_cancel(&self) {
        self.cancel_requested.store(true, Ordering::Relaxed);
    }
}

#[derive(Debug)]
pub(super) struct SearchProgressMessage {
    pub(super) tab_id: TabId,
    pub(super) run_id: SearchRunId,
    pub(super) progress: SearchProgress,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SearchProgressKey {
    tab_id: TabId,
    run_id: SearchRunId,
}

impl SearchProgressKey {
    fn from_message(message: &SearchProgressMessage) -> Self {
        Self {
            tab_id: message.tab_id,
            run_id: message.run_id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SearchProgressInsertion {
    pub(super) token: ui::MessageLong,
    pub(super) should_post: bool,
}

#[derive(Debug)]
pub(super) struct SearchCompleteMessage {
    pub(super) tab_id: TabId,
    pub(super) run_id: SearchRunId,
    pub(super) result: ExplorerResult<SearchOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ListingRequest {
    pub(super) generation: u64,
    pub(super) tab_id: TabId,
    pub(super) location: NavigationLocation,
    pub(super) display_options: DisplayOptions,
    pub(super) sort: SortState,
}

impl ListingRequest {
    pub(super) fn has_same_listing_source(
        &self,
        tab_id: TabId,
        location: &NavigationLocation,
        display_options: DisplayOptions,
    ) -> bool {
        self.tab_id == tab_id
            && self.location.has_same_path(location.as_path())
            && self.display_options == display_options
    }

    pub(super) fn has_same_listing_source_as(&self, other: &Self) -> bool {
        self.has_same_listing_source(other.tab_id, &other.location, other.display_options)
    }
}

#[derive(Debug)]
pub(super) struct ActiveListingWorker {
    pub(super) request: ListingRequest,
    pub(super) cancel_requested: Arc<AtomicBool>,
    pub(super) io_cancellation: Arc<SynchronousIoCancellation>,
    pub(super) handle: Option<JoinHandle<()>>,
}

impl ActiveListingWorker {
    pub(super) fn request_cancel(&self) {
        self.cancel_requested.store(true, Ordering::Relaxed);
        if let Err(error) = self.io_cancellation.request_cancel() {
            eprintln!(
                "listing worker for tab {:?}, generation {} cancellation failed: {error}",
                self.request.tab_id, self.request.generation
            );
        }
    }

    pub(super) fn is_cancel_requested(&self) -> bool {
        self.cancel_requested.load(Ordering::Relaxed)
    }

    pub(super) fn is_finished(&self) -> bool {
        self.handle
            .as_ref()
            .map(|handle| handle.is_finished())
            .unwrap_or(true)
    }
}

#[derive(Debug)]
pub(super) struct ListingCompleteMessage {
    pub(super) request: ListingRequest,
    pub(super) result: ExplorerResult<Vec<FileItem>>,
}

#[derive(Debug)]
pub(super) struct ActiveFileWatchWorker {
    pub(super) generation: u64,
    pub(super) location: NavigationLocation,
    pub(super) cancel_requested: Arc<AtomicBool>,
    pub(super) cancellation: Arc<DirectoryChangeCancellation>,
    pub(super) handle: Option<JoinHandle<()>>,
}

impl ActiveFileWatchWorker {
    pub(super) fn request_cancel(&self) {
        self.cancel_requested.store(true, Ordering::Relaxed);
        if let Err(error) = self.cancellation.request_cancel() {
            eprintln!(
                "failed to request file watch cancellation for generation {}: {error}",
                self.generation
            );
        }
    }

    pub(super) fn is_cancel_requested(&self) -> bool {
        self.cancel_requested.load(Ordering::Relaxed)
    }

    pub(super) fn is_finished(&self) -> bool {
        self.handle
            .as_ref()
            .map(|handle| handle.is_finished())
            .unwrap_or(true)
    }
}

#[derive(Debug)]
pub(super) struct FileWatchChangeMessage {
    pub(super) generation: u64,
    pub(super) changes: DirectoryChangeBatch,
}

#[derive(Debug)]
pub(super) enum FileOperationRequest {
    Transfer {
        tab_id: TabId,
        location: NavigationLocation,
        operation: DropOperation,
        sources: Vec<NavigationLocation>,
        destination: NavigationLocation,
        select_completed_items: bool,
    },
    Delete {
        tab_id: TabId,
        location: NavigationLocation,
        operation: DeleteFileOperation,
        targets: Vec<NavigationLocation>,
    },
    Rename {
        tab_id: TabId,
        location: NavigationLocation,
        target: NavigationLocation,
        new_name: OsString,
        undo_original_name: Option<OsString>,
    },
    UndoMove {
        tab_id: TabId,
        location: NavigationLocation,
        moved: Vec<(NavigationLocation, NavigationLocation)>,
    },
}

impl FileOperationRequest {
    pub(super) fn tab_id(&self) -> TabId {
        match self {
            Self::Transfer { tab_id, .. }
            | Self::Delete { tab_id, .. }
            | Self::Rename { tab_id, .. }
            | Self::UndoMove { tab_id, .. } => *tab_id,
        }
    }

    pub(super) fn location(&self) -> &NavigationLocation {
        match self {
            Self::Transfer { location, .. }
            | Self::Delete { location, .. }
            | Self::Rename { location, .. }
            | Self::UndoMove { location, .. } => location,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DeleteFileOperation {
    ToRecycleBin,
    Permanently,
}

#[derive(Debug)]
pub(super) struct ActiveFileOperationWorker {
    pub(super) generation: u64,
    pub(super) tab_id: TabId,
    pub(super) handle: Option<JoinHandle<()>>,
}

impl ActiveFileOperationWorker {
    pub(super) fn is_finished(&self) -> bool {
        self.handle
            .as_ref()
            .map(|handle| handle.is_finished())
            .unwrap_or(true)
    }
}

#[derive(Debug)]
pub(super) struct FileOperationWorkerOutcome {
    pub(super) affected_folders: Vec<NavigationLocation>,
    pub(super) selected_items: Vec<NavigationLocation>,
    pub(super) undo_file_operation: Option<UndoFileOperation>,
    pub(super) completion_error: Option<ExplorerError>,
}

#[derive(Debug)]
pub(super) struct FileOperationCompleteMessage {
    pub(super) generation: u64,
    pub(super) tab_id: TabId,
    pub(super) location: NavigationLocation,
    pub(super) result: ExplorerResult<FileOperationWorkerOutcome>,
}

#[derive(Debug)]
pub(super) struct WorkerController {
    pub(super) messages: WorkerMessageStore,
    pub(super) search_shutdown_requested: Arc<AtomicBool>,
    pub(super) search_workers: Vec<ActiveSearchWorker>,
    pub(super) pending_search_workers: Vec<PendingSearchWorker>,
    pub(super) search_completion_timer_active: bool,
    pub(super) listing_shutdown_requested: Arc<AtomicBool>,
    pub(super) listing_generation: u64,
    pub(super) listing_worker: Option<ActiveListingWorker>,
    pub(super) retired_listing_workers: Vec<ActiveListingWorker>,
    pub(super) pending_listing_request: Option<ListingRequest>,
    pub(super) file_watch_generation: u64,
    pub(super) file_watch_worker: Option<ActiveFileWatchWorker>,
    pub(super) retired_file_watch_workers: Vec<ActiveFileWatchWorker>,
    pub(super) file_watch_refresh_timer_active: bool,
    pub(super) file_operation_generation: u64,
    pub(super) file_operation_worker: Option<ActiveFileOperationWorker>,
}

impl WorkerController {
    pub(super) fn new() -> Self {
        Self {
            messages: WorkerMessageStore::new(),
            search_shutdown_requested: Arc::new(AtomicBool::new(false)),
            search_workers: Vec::new(),
            pending_search_workers: Vec::new(),
            search_completion_timer_active: false,
            listing_shutdown_requested: Arc::new(AtomicBool::new(false)),
            listing_generation: 0,
            listing_worker: None,
            retired_listing_workers: Vec::new(),
            pending_listing_request: None,
            file_watch_generation: 0,
            file_watch_worker: None,
            retired_file_watch_workers: Vec::new(),
            file_watch_refresh_timer_active: false,
            file_operation_generation: 0,
            file_operation_worker: None,
        }
    }

    pub(super) fn next_listing_generation(&mut self) -> u64 {
        self.listing_generation = if self.listing_generation == u64::MAX {
            1
        } else {
            self.listing_generation + 1
        };
        self.listing_generation
    }

    pub(super) fn next_file_operation_generation(&mut self) -> u64 {
        self.file_operation_generation = if self.file_operation_generation == u64::MAX {
            1
        } else {
            self.file_operation_generation + 1
        };
        self.file_operation_generation
    }

    pub(super) fn next_file_watch_generation(&mut self) -> u64 {
        self.file_watch_generation = if self.file_watch_generation >= isize::MAX as u64 {
            1
        } else {
            self.file_watch_generation + 1
        };
        self.file_watch_generation
    }

    pub(super) fn search_shutdown_requested(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.search_shutdown_requested)
    }

    pub(super) fn listing_shutdown_requested(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.listing_shutdown_requested)
    }

    pub(super) fn reap_finished_search_workers(&mut self) {
        reap_finished_search_workers(&mut self.search_workers);
    }

    pub(super) fn detach_cancelled_search_workers(&mut self) {
        detach_cancelled_search_workers(&mut self.search_workers);
    }

    pub(super) fn cancel_running_search_workers_for_tab(&self, tab_id: TabId) {
        for worker in self
            .search_workers
            .iter()
            .filter(|worker| worker.tab_id == tab_id)
        {
            worker.request_cancel();
        }
    }

    pub(super) fn cancel_pending_search_workers_for_tab(
        &mut self,
        tab_id: TabId,
    ) -> Vec<PendingSearchWorker> {
        let pending_workers =
            take_pending_search_workers_for_tab(&mut self.pending_search_workers, tab_id);
        for worker in &pending_workers {
            worker.request_cancel();
        }
        pending_workers
    }

    pub(super) fn has_running_search_worker_for_tab(&self, tab_id: TabId) -> bool {
        self.search_workers
            .iter()
            .any(|worker| worker.tab_id == tab_id)
    }

    pub(super) fn replace_pending_search_worker(&mut self, pending_worker: PendingSearchWorker) {
        replace_pending_search_worker(&mut self.pending_search_workers, pending_worker);
    }

    pub(super) fn push_search_worker(
        &mut self,
        tab_id: TabId,
        run_id: SearchRunId,
        cancel_requested: Arc<AtomicBool>,
        io_cancellation: Arc<SynchronousIoCancellation>,
        handle: JoinHandle<()>,
    ) {
        self.search_workers.push(ActiveSearchWorker {
            tab_id,
            run_id,
            cancel_requested,
            io_cancellation,
            handle: Some(handle),
        });
    }

    pub(super) fn take_pending_search_worker_for_tab(
        &mut self,
        tab_id: TabId,
    ) -> Option<PendingSearchWorker> {
        let position = self
            .pending_search_workers
            .iter()
            .position(|worker| worker.tab_id() == tab_id)?;
        Some(self.pending_search_workers.remove(position))
    }

    pub(super) fn remove_search_worker(&mut self, tab_id: TabId, run_id: SearchRunId) {
        if let Some(position) = self
            .search_workers
            .iter()
            .position(|worker| worker.tab_id == tab_id && worker.run_id == run_id)
        {
            join_search_worker(self.search_workers.remove(position));
        }
    }

    pub(super) fn has_active_search_work(&self) -> bool {
        !self.search_workers.is_empty() || !self.pending_search_workers.is_empty()
    }

    pub(super) fn cancel_searches_for_shutdown(&mut self) {
        self.search_shutdown_requested
            .store(true, Ordering::Relaxed);
        for worker in self.pending_search_workers.drain(..) {
            worker.request_cancel();
        }
        cancel_search_workers(&mut self.search_workers);
        self.messages.clear_search();
    }

    pub(super) fn active_listing_request(&self) -> Option<&ListingRequest> {
        self.listing_worker.as_ref().map(|worker| &worker.request)
    }

    pub(super) fn active_uncancelled_listing_request(&self) -> Option<&ListingRequest> {
        self.listing_worker
            .as_ref()
            .filter(|worker| !worker.is_cancel_requested())
            .map(|worker| &worker.request)
    }

    pub(super) fn active_listing_matches_source(&self, request: &ListingRequest) -> bool {
        self.listing_worker.as_ref().is_some_and(|worker| {
            !worker.is_cancel_requested() && worker.request.has_same_listing_source_as(request)
        })
    }

    pub(super) fn reap_retired_listing_workers(&mut self) {
        reap_finished_listing_workers(&mut self.retired_listing_workers);
    }

    pub(super) fn retire_active_listing_worker(&mut self) -> bool {
        self.reap_retired_listing_workers();
        retire_listing_worker(&mut self.listing_worker, &mut self.retired_listing_workers)
    }

    pub(super) fn has_active_listing_worker(&self) -> bool {
        self.listing_worker.is_some()
    }

    pub(super) fn replace_pending_listing_request(&mut self, request: ListingRequest) {
        self.pending_listing_request = Some(request);
    }

    pub(super) fn clear_pending_listing_request(&mut self) {
        self.pending_listing_request = None;
    }

    pub(super) fn take_pending_listing_request(&mut self) -> Option<ListingRequest> {
        self.pending_listing_request.take()
    }

    pub(super) fn start_listing_worker(
        &mut self,
        request: ListingRequest,
        cancel_requested: Arc<AtomicBool>,
        io_cancellation: Arc<SynchronousIoCancellation>,
        handle: JoinHandle<()>,
    ) {
        self.listing_worker = Some(ActiveListingWorker {
            request,
            cancel_requested,
            io_cancellation,
            handle: Some(handle),
        });
    }

    pub(super) fn finish_listing_worker_for_generation(&mut self, generation: u64) {
        if self
            .listing_worker
            .as_ref()
            .is_some_and(|worker| worker.request.generation == generation)
        {
            self.finish_active_listing_worker();
        }
    }

    pub(super) fn is_current_listing_generation(&self, generation: u64) -> bool {
        generation == self.listing_generation
    }

    pub(super) fn finish_active_listing_worker(&mut self) {
        if let Some(worker) = self.listing_worker.take() {
            join_listing_worker(worker);
        }
    }

    pub(super) fn active_file_watch_matches(&self, location: &NavigationLocation) -> bool {
        self.file_watch_worker.as_ref().is_some_and(|worker| {
            !worker.is_cancel_requested() && worker.location.has_same_path(location.as_path())
        })
    }

    pub(super) fn has_active_file_watch_worker(&self) -> bool {
        self.file_watch_worker.is_some()
    }

    pub(super) fn start_file_watch_worker(
        &mut self,
        generation: u64,
        location: NavigationLocation,
        cancellation: Arc<DirectoryChangeCancellation>,
        handle: JoinHandle<()>,
    ) {
        self.file_watch_worker = Some(ActiveFileWatchWorker {
            generation,
            location,
            cancel_requested: Arc::new(AtomicBool::new(false)),
            cancellation,
            handle: Some(handle),
        });
    }

    pub(super) fn retire_active_file_watch_worker(&mut self) -> bool {
        self.reap_retired_file_watch_workers();
        retire_file_watch_worker(
            &mut self.file_watch_worker,
            &mut self.retired_file_watch_workers,
        )
    }

    pub(super) fn reap_retired_file_watch_workers(&mut self) {
        reap_finished_file_watch_workers(&mut self.retired_file_watch_workers);
    }

    pub(super) fn is_current_file_watch_generation(&self, generation: u64) -> bool {
        self.file_watch_worker
            .as_ref()
            .is_some_and(|worker| worker.generation == generation && !worker.is_cancel_requested())
    }

    pub(super) fn has_file_operation_worker(&self) -> bool {
        self.file_operation_worker.is_some()
    }

    pub(super) fn ensure_file_operation_worker_idle(&self) -> ExplorerResult<()> {
        if self.has_file_operation_worker() {
            Err(ExplorerError::invalid_input(
                "다른 파일 작업이 아직 완료되지 않았습니다.",
            ))
        } else {
            Ok(())
        }
    }

    pub(super) fn start_file_operation_worker(
        &mut self,
        generation: u64,
        tab_id: TabId,
        handle: JoinHandle<()>,
    ) {
        self.file_operation_worker = Some(ActiveFileOperationWorker {
            generation,
            tab_id,
            handle: Some(handle),
        });
    }

    pub(super) fn finish_file_operation_worker_for_generation(&mut self, generation: u64) {
        if self
            .file_operation_worker
            .as_ref()
            .is_some_and(|worker| worker.generation == generation)
        {
            join_file_operation_worker(&mut self.file_operation_worker);
        }
    }

    pub(super) fn reap_finished_file_operation_worker_for_shutdown(&mut self) {
        if self
            .file_operation_worker
            .as_ref()
            .is_some_and(ActiveFileOperationWorker::is_finished)
        {
            join_file_operation_worker(&mut self.file_operation_worker);
        }
    }

    pub(super) fn is_current_file_operation_generation(&self, generation: u64) -> bool {
        generation == self.file_operation_generation
    }

    pub(super) fn cleanup_background_workers_for_shutdown(&mut self) {
        self.listing_shutdown_requested
            .store(true, Ordering::Relaxed);
        cancel_listing_workers_for_shutdown(
            &mut self.listing_worker,
            &mut self.retired_listing_workers,
        );
        cancel_file_watch_workers_for_shutdown(
            &mut self.file_watch_worker,
            &mut self.retired_file_watch_workers,
        );
        join_file_operation_worker(&mut self.file_operation_worker);
        self.pending_listing_request = None;
        self.messages.clear_listing();
        self.messages.clear_file_watch();
    }
}

pub(super) fn retire_listing_worker(
    listing_worker: &mut Option<ActiveListingWorker>,
    retired_listing_workers: &mut Vec<ActiveListingWorker>,
) -> bool {
    let Some(worker) = listing_worker.as_ref() else {
        return true;
    };

    if retired_listing_workers.len() >= MAX_RETIRED_LISTING_WORKERS {
        if worker.is_finished() {
            if let Some(worker) = listing_worker.take() {
                join_listing_worker(worker);
            }
            return true;
        }

        if !worker.is_cancel_requested() {
            worker.request_cancel();
        }
        return false;
    }

    let Some(worker) = listing_worker.take() else {
        return true;
    };
    worker.request_cancel();
    retired_listing_workers.push(worker);
    true
}

pub(super) fn reap_finished_listing_workers(listing_workers: &mut Vec<ActiveListingWorker>) {
    let mut index = 0;
    while index < listing_workers.len() {
        if listing_workers[index].is_finished() {
            join_listing_worker(listing_workers.remove(index));
        } else {
            index += 1;
        }
    }
}

pub(super) fn join_listing_worker(mut worker: ActiveListingWorker) {
    let Some(handle) = worker.handle.take() else {
        return;
    };

    if handle.join().is_err() {
        eprintln!(
            "listing worker for tab {:?}, generation {} panicked before completion",
            worker.request.tab_id, worker.request.generation
        );
    }
}

pub(super) fn cancel_listing_workers_for_shutdown(
    listing_worker: &mut Option<ActiveListingWorker>,
    retired_listing_workers: &mut Vec<ActiveListingWorker>,
) {
    if let Some(mut worker) = listing_worker.take() {
        detach_listing_worker(&mut worker);
    }

    for mut worker in retired_listing_workers.drain(..) {
        detach_listing_worker(&mut worker);
    }
}

fn detach_listing_worker(worker: &mut ActiveListingWorker) {
    worker.request_cancel();
    // Dropping JoinHandle detaches the thread; cancellation state is owned by the worker thread.
    drop(worker.handle.take());
}

pub(super) fn retire_file_watch_worker(
    file_watch_worker: &mut Option<ActiveFileWatchWorker>,
    retired_file_watch_workers: &mut Vec<ActiveFileWatchWorker>,
) -> bool {
    let Some(worker) = file_watch_worker.as_ref() else {
        return true;
    };

    if retired_file_watch_workers.len() >= MAX_RETIRED_FILE_WATCH_WORKERS {
        if worker.is_finished() {
            if let Some(worker) = file_watch_worker.take() {
                join_file_watch_worker(worker);
            }
            return true;
        }

        if !worker.is_cancel_requested() {
            worker.request_cancel();
        }
        return false;
    }

    let Some(worker) = file_watch_worker.take() else {
        return true;
    };
    worker.request_cancel();
    retired_file_watch_workers.push(worker);
    true
}

pub(super) fn reap_finished_file_watch_workers(
    file_watch_workers: &mut Vec<ActiveFileWatchWorker>,
) {
    let mut index = 0;
    while index < file_watch_workers.len() {
        if file_watch_workers[index].is_finished() {
            join_file_watch_worker(file_watch_workers.remove(index));
        } else {
            index += 1;
        }
    }
}

pub(super) fn join_file_watch_worker(mut worker: ActiveFileWatchWorker) {
    let Some(handle) = worker.handle.take() else {
        return;
    };

    if handle.join().is_err() {
        eprintln!(
            "file watch worker for generation {} at {:?} panicked before shutdown",
            worker.generation,
            worker.location.as_path()
        );
    }
}

pub(super) fn cancel_file_watch_workers_for_shutdown(
    file_watch_worker: &mut Option<ActiveFileWatchWorker>,
    retired_file_watch_workers: &mut Vec<ActiveFileWatchWorker>,
) {
    if let Some(mut worker) = file_watch_worker.take() {
        detach_file_watch_worker_for_shutdown(&mut worker);
    }

    for mut worker in retired_file_watch_workers.drain(..) {
        detach_file_watch_worker_for_shutdown(&mut worker);
    }
}

fn detach_file_watch_worker_for_shutdown(worker: &mut ActiveFileWatchWorker) {
    worker.request_cancel();
    // Dropping JoinHandle detaches the thread; shutdown must not wait on network file watchers.
    drop(worker.handle.take());
}

pub(super) fn reap_finished_search_workers(search_workers: &mut Vec<ActiveSearchWorker>) {
    let mut index = 0;
    while index < search_workers.len() {
        if search_workers[index].is_finished() {
            join_search_worker(search_workers.remove(index));
        } else {
            index += 1;
        }
    }
}

pub(super) fn detach_cancelled_search_workers(search_workers: &mut Vec<ActiveSearchWorker>) {
    let mut index = 0;
    while index < search_workers.len() {
        if search_workers[index].is_cancel_requested() {
            detach_search_worker(search_workers.remove(index));
        } else {
            index += 1;
        }
    }
}

pub(super) fn cancel_search_workers(search_workers: &mut Vec<ActiveSearchWorker>) {
    for worker in search_workers.drain(..) {
        detach_search_worker(worker);
    }
}

fn detach_search_worker(mut worker: ActiveSearchWorker) {
    worker.request_cancel();
    // Dropping JoinHandle detaches the thread; cancelled synchronous search I/O must not occupy UI search slots.
    drop(worker.handle.take());
}

pub(super) fn join_search_worker(mut worker: ActiveSearchWorker) {
    let Some(handle) = worker.handle.take() else {
        return;
    };

    if handle.join().is_err() {
        eprintln!(
            "search worker for tab {:?}, run {:?} panicked before completion",
            worker.tab_id, worker.run_id
        );
    }
}

pub(super) fn join_file_operation_worker(worker: &mut Option<ActiveFileOperationWorker>) {
    let Some(mut worker) = worker.take() else {
        return;
    };
    let Some(handle) = worker.handle.take() else {
        return;
    };

    if handle.join().is_err() {
        eprintln!(
            "file operation worker for tab {:?}, generation {} panicked before completion",
            worker.tab_id, worker.generation
        );
    }
}

pub(super) fn take_pending_search_workers_for_tab(
    pending_workers: &mut Vec<PendingSearchWorker>,
    tab_id: TabId,
) -> Vec<PendingSearchWorker> {
    let mut matching_workers = Vec::new();
    let mut index = 0;
    while index < pending_workers.len() {
        if pending_workers[index].tab_id() == tab_id {
            matching_workers.push(pending_workers.remove(index));
        } else {
            index += 1;
        }
    }
    matching_workers
}

pub(super) fn replace_pending_search_worker(
    pending_workers: &mut Vec<PendingSearchWorker>,
    pending_worker: PendingSearchWorker,
) {
    for worker in take_pending_search_workers_for_tab(pending_workers, pending_worker.tab_id()) {
        worker.request_cancel();
    }
    pending_workers.push(pending_worker);
}

#[derive(Debug, Default)]
pub(super) struct PendingSearchMessages {
    pub(super) next_token: ui::MessageLong,
    pub(super) progress: HashMap<ui::MessageLong, SearchProgressMessage>,
    progress_tokens: HashMap<SearchProgressKey, ui::MessageLong>,
    pub(super) complete: HashMap<ui::MessageLong, SearchCompleteMessage>,
}

impl PendingSearchMessages {
    pub(super) fn insert_progress(
        &mut self,
        message: SearchProgressMessage,
    ) -> SearchProgressInsertion {
        let key = SearchProgressKey::from_message(&message);
        if let Some(token) = self.progress_tokens.get(&key).copied() {
            if let Some(pending_message) = self.progress.get_mut(&token) {
                *pending_message = message;
                return SearchProgressInsertion {
                    token,
                    should_post: false,
                };
            }
            self.progress_tokens.remove(&key);
        }

        let token = self.next_available_token();
        self.progress.insert(token, message);
        self.progress_tokens.insert(key, token);
        SearchProgressInsertion {
            token,
            should_post: true,
        }
    }

    pub(super) fn insert_complete(&mut self, message: SearchCompleteMessage) -> ui::MessageLong {
        let token = self.next_available_token();
        self.complete.insert(token, message);
        token
    }

    fn take_progress(&mut self, token: ui::MessageLong) -> Option<SearchProgressMessage> {
        let message = self.progress.remove(&token)?;
        let key = SearchProgressKey::from_message(&message);
        if self.progress_tokens.get(&key).copied() == Some(token) {
            self.progress_tokens.remove(&key);
        }
        Some(message)
    }

    fn take_complete(&mut self, token: ui::MessageLong) -> Option<SearchCompleteMessage> {
        self.complete.remove(&token)
    }

    fn take_next_complete(&mut self) -> Option<SearchCompleteMessage> {
        let token = self.complete.keys().copied().next()?;
        self.complete.remove(&token)
    }

    fn has_complete(&self) -> bool {
        !self.complete.is_empty()
    }

    fn clear(&mut self) {
        self.progress.clear();
        self.progress_tokens.clear();
        self.complete.clear();
    }

    fn next_available_token(&mut self) -> ui::MessageLong {
        loop {
            self.next_token = next_worker_message_token(self.next_token);
            let token = self.next_token;
            if !self.progress.contains_key(&token) && !self.complete.contains_key(&token) {
                return token;
            }
        }
    }
}

fn next_worker_message_token(current: ui::MessageLong) -> ui::MessageLong {
    if (0..isize::MAX).contains(&current) {
        current + 1
    } else {
        1
    }
}

fn keep_pending_complete_message_for_timer(
    hwnd: ui::WindowHandle,
    message_kind: &str,
    error: impl std::fmt::Display,
) -> bool {
    eprintln!("failed to post {message_kind} complete message; keeping pending result: {error}");
    match ui::set_window_timer(hwnd, ID_SEARCH_COMPLETION_TIMER, SEARCH_COMPLETION_POLL_MS) {
        Ok(()) => true,
        Err(timer_error) => {
            eprintln!("failed to start {message_kind} completion recovery timer: {timer_error}");
            false
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct PendingListingMessages {
    pub(super) next_token: ui::MessageLong,
    pub(super) complete: HashMap<ui::MessageLong, ListingCompleteMessage>,
}

impl PendingListingMessages {
    fn insert_complete(&mut self, message: ListingCompleteMessage) -> ui::MessageLong {
        let token = self.next_available_token();
        self.complete.insert(token, message);
        token
    }

    fn take_complete(&mut self, token: ui::MessageLong) -> Option<ListingCompleteMessage> {
        self.complete.remove(&token)
    }

    fn take_next_complete(&mut self) -> Option<ListingCompleteMessage> {
        let token = self.complete.keys().copied().next()?;
        self.complete.remove(&token)
    }

    fn has_complete(&self) -> bool {
        !self.complete.is_empty()
    }

    fn clear(&mut self) {
        self.complete.clear();
    }

    fn next_available_token(&mut self) -> ui::MessageLong {
        loop {
            self.next_token = next_worker_message_token(self.next_token);
            let token = self.next_token;
            if !self.complete.contains_key(&token) {
                return token;
            }
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct PendingFileOperationMessages {
    pub(super) next_token: ui::MessageLong,
    pub(super) complete: HashMap<ui::MessageLong, FileOperationCompleteMessage>,
}

impl PendingFileOperationMessages {
    fn insert_complete(&mut self, message: FileOperationCompleteMessage) -> ui::MessageLong {
        let token = self.next_available_token();
        self.complete.insert(token, message);
        token
    }

    fn take_complete(&mut self, token: ui::MessageLong) -> Option<FileOperationCompleteMessage> {
        self.complete.remove(&token)
    }

    fn take_next_complete(&mut self) -> Option<FileOperationCompleteMessage> {
        let token = self.complete.keys().copied().next()?;
        self.complete.remove(&token)
    }

    fn has_complete(&self) -> bool {
        !self.complete.is_empty()
    }

    fn next_available_token(&mut self) -> ui::MessageLong {
        loop {
            self.next_token = next_worker_message_token(self.next_token);
            let token = self.next_token;
            if !self.complete.contains_key(&token) {
                return token;
            }
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct PendingFileWatchMessages {
    pub(super) next_token: ui::MessageLong,
    pub(super) changed: HashMap<ui::MessageLong, FileWatchChangeMessage>,
}

impl PendingFileWatchMessages {
    fn insert_changed(&mut self, message: FileWatchChangeMessage) -> ui::MessageLong {
        let token = self.next_available_token();
        self.changed.insert(token, message);
        token
    }

    fn take_changed(&mut self, token: ui::MessageLong) -> Option<FileWatchChangeMessage> {
        self.changed.remove(&token)
    }

    fn take_next_changed(&mut self) -> Option<FileWatchChangeMessage> {
        let token = self.changed.keys().copied().next()?;
        self.changed.remove(&token)
    }

    fn has_changed(&self) -> bool {
        !self.changed.is_empty()
    }

    fn clear(&mut self) {
        self.changed.clear();
    }

    fn next_available_token(&mut self) -> ui::MessageLong {
        loop {
            self.next_token = next_worker_message_token(self.next_token);
            let token = self.next_token;
            if !self.changed.contains_key(&token) {
                return token;
            }
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct PendingWorkerMessages {
    pub(super) search: PendingSearchMessages,
    pub(super) listing: PendingListingMessages,
    pub(super) file_operation: PendingFileOperationMessages,
    pub(super) file_watch: PendingFileWatchMessages,
    pub(super) completion_recovery_requested: bool,
}

#[derive(Debug, Clone)]
pub(super) struct WorkerMessageStore {
    inner: Arc<Mutex<PendingWorkerMessages>>,
}

impl WorkerMessageStore {
    pub(super) fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(PendingWorkerMessages::default())),
        }
    }

    pub(super) fn lock(&self) -> MutexGuard<'_, PendingWorkerMessages> {
        match self.inner.lock() {
            Ok(messages) => messages,
            Err(poisoned) => {
                eprintln!("worker message store lock was poisoned; continuing with pending state");
                poisoned.into_inner()
            }
        }
    }

    pub(super) fn clear_search(&self) {
        self.lock().search.clear();
    }

    pub(super) fn clear_listing(&self) {
        self.lock().listing.clear();
    }

    pub(super) fn clear_file_watch(&self) {
        self.lock().file_watch.clear();
    }

    fn request_completion_recovery(&self) {
        self.lock().completion_recovery_requested = true;
    }

    pub(super) fn post_search_progress(&self, hwnd_value: isize, message: SearchProgressMessage) {
        let insertion = {
            let mut messages = self.lock();
            messages.search.insert_progress(message)
        };
        if insertion.should_post {
            self.post_pending_message(
                hwnd_value,
                MESSAGE_SEARCH_PROGRESS,
                insertion.token,
                |messages| {
                    let _ = messages.search.take_progress(insertion.token);
                },
            );
        }
    }

    pub(super) fn post_search_complete(&self, hwnd_value: isize, message: SearchCompleteMessage) {
        let token = {
            let mut messages = self.lock();
            messages.search.insert_complete(message)
        };
        self.post_complete_message(hwnd_value, MESSAGE_SEARCH_COMPLETE, token, "search");
    }

    pub(super) fn post_listing_complete(&self, hwnd_value: isize, message: ListingCompleteMessage) {
        let token = {
            let mut messages = self.lock();
            messages.listing.insert_complete(message)
        };
        self.post_complete_message(hwnd_value, MESSAGE_LISTING_COMPLETE, token, "listing");
    }

    pub(super) fn post_file_operation_complete(
        &self,
        hwnd_value: isize,
        message: FileOperationCompleteMessage,
    ) {
        let token = {
            let mut messages = self.lock();
            messages.file_operation.insert_complete(message)
        };
        self.post_complete_message(
            hwnd_value,
            MESSAGE_FILE_OPERATION_COMPLETE,
            token,
            "file operation",
        );
    }

    pub(super) fn post_file_watch_changed(
        &self,
        hwnd_value: isize,
        message: FileWatchChangeMessage,
    ) {
        let token = {
            let mut messages = self.lock();
            messages.file_watch.insert_changed(message)
        };
        let hwnd = ui::WindowHandle::from_isize(hwnd_value);
        if let Err(error) = ui::post_window_message(hwnd, MESSAGE_FILE_WATCH_CHANGED, 0, token) {
            eprintln!("failed to post file watch change message; keeping pending changes: {error}");
            if let Err(timer_error) = ui::set_window_timer(
                hwnd,
                ID_FILE_WATCH_REFRESH_TIMER,
                FILE_WATCH_REFRESH_DEBOUNCE_MS,
            ) {
                eprintln!("failed to start file watch change recovery timer: {timer_error}");
            }
        }
    }

    fn post_pending_message(
        &self,
        hwnd_value: isize,
        message: u32,
        token: ui::MessageLong,
        rollback: impl FnOnce(&mut PendingWorkerMessages),
    ) {
        let hwnd = ui::WindowHandle::from_isize(hwnd_value);
        if ui::post_window_message(hwnd, message, 0, token).is_err() {
            let mut messages = self.lock();
            rollback(&mut messages);
        }
    }

    fn post_complete_message(
        &self,
        hwnd_value: isize,
        message: u32,
        token: ui::MessageLong,
        message_kind: &str,
    ) {
        let hwnd = ui::WindowHandle::from_isize(hwnd_value);
        if let Err(error) = ui::post_window_message(hwnd, message, 0, token) {
            if !keep_pending_complete_message_for_timer(hwnd, message_kind, error) {
                self.request_completion_recovery();
            }
        }
    }

    pub(super) fn take_search_progress(
        &self,
        lparam: ui::MessageLong,
    ) -> Option<SearchProgressMessage> {
        if lparam == 0 {
            return None;
        }
        self.lock().search.take_progress(lparam)
    }

    pub(super) fn take_search_complete(
        &self,
        lparam: ui::MessageLong,
    ) -> Option<SearchCompleteMessage> {
        if lparam == 0 {
            return None;
        }
        self.lock().search.take_complete(lparam)
    }

    pub(super) fn take_next_search_complete(&self) -> Option<SearchCompleteMessage> {
        self.lock().search.take_next_complete()
    }

    pub(super) fn take_listing_complete(
        &self,
        lparam: ui::MessageLong,
    ) -> Option<ListingCompleteMessage> {
        if lparam == 0 {
            return None;
        }
        self.lock().listing.take_complete(lparam)
    }

    pub(super) fn take_next_listing_complete(&self) -> Option<ListingCompleteMessage> {
        self.lock().listing.take_next_complete()
    }

    pub(super) fn take_file_operation_complete(
        &self,
        lparam: ui::MessageLong,
    ) -> Option<FileOperationCompleteMessage> {
        if lparam == 0 {
            return None;
        }
        self.lock().file_operation.take_complete(lparam)
    }

    pub(super) fn take_file_watch_changed(
        &self,
        lparam: ui::MessageLong,
    ) -> Option<FileWatchChangeMessage> {
        if lparam == 0 {
            return None;
        }
        self.lock().file_watch.take_changed(lparam)
    }

    pub(super) fn take_next_file_watch_changed(&self) -> Option<FileWatchChangeMessage> {
        self.lock().file_watch.take_next_changed()
    }

    pub(super) fn take_next_file_operation_complete(&self) -> Option<FileOperationCompleteMessage> {
        self.lock().file_operation.take_next_complete()
    }

    pub(super) fn has_pending_file_watch_changed(&self) -> bool {
        self.lock().file_watch.has_changed()
    }

    pub(super) fn has_pending_complete(&self) -> bool {
        let messages = self.lock();
        messages.search.has_complete()
            || messages.listing.has_complete()
            || messages.file_operation.has_complete()
    }

    pub(super) fn take_completion_recovery_request(&self) -> bool {
        let mut messages = self.lock();
        let requested = messages.completion_recovery_requested;
        messages.completion_recovery_requested = false;
        requested
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    };
    use std::thread;
    use std::time::Duration;

    use j3files::domain::{ExplorerResult, NavigationLocation};
    use j3files::platform::DirectoryChangeCancellation;

    use super::{
        join_file_watch_worker, reap_finished_file_watch_workers, retire_file_watch_worker,
        ActiveFileWatchWorker,
    };

    fn file_watch_location(generation: u64) -> NavigationLocation {
        NavigationLocation::LocalPath(PathBuf::from(format!(r"C:\watch\{generation}")))
    }

    fn blocked_file_watch_worker(
        generation: u64,
    ) -> ExplorerResult<(ActiveFileWatchWorker, mpsc::Sender<()>)> {
        let (release_tx, release_rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let _ = release_rx.recv_timeout(Duration::from_secs(1));
        });

        Ok((
            ActiveFileWatchWorker {
                generation,
                location: file_watch_location(generation),
                cancel_requested: Arc::new(AtomicBool::new(false)),
                cancellation: Arc::new(DirectoryChangeCancellation::new()?),
                handle: Some(handle),
            },
            release_tx,
        ))
    }

    #[test]
    fn retiring_file_watch_worker_requests_cancel_and_reaps_finished_handle() -> ExplorerResult<()>
    {
        let (active_worker, release) = blocked_file_watch_worker(1)?;
        let cancel_requested = Arc::clone(&active_worker.cancel_requested);
        let cancellation = Arc::clone(&active_worker.cancellation);
        let mut file_watch_worker = Some(active_worker);
        let mut retired_file_watch_workers = Vec::new();

        assert!(retire_file_watch_worker(
            &mut file_watch_worker,
            &mut retired_file_watch_workers,
        ));

        assert!(file_watch_worker.is_none());
        assert!(cancel_requested.load(Ordering::Relaxed));
        assert!(cancellation.is_cancel_requested()?);
        assert_eq!(retired_file_watch_workers.len(), 1);

        let _ = release.send(());
        while retired_file_watch_workers
            .iter()
            .any(|worker| !worker.is_finished())
        {
            thread::yield_now();
        }
        reap_finished_file_watch_workers(&mut retired_file_watch_workers);

        assert!(retired_file_watch_workers.is_empty());
        Ok(())
    }

    #[test]
    fn retiring_file_watch_worker_at_capacity_defers_without_detaching_running_worker(
    ) -> ExplorerResult<()> {
        let (retired_worker, retired_release) = blocked_file_watch_worker(1)?;
        retired_worker.request_cancel();
        let retired_cancel_requested = Arc::clone(&retired_worker.cancel_requested);
        let (active_worker, active_release) = blocked_file_watch_worker(2)?;
        let active_cancel_requested = Arc::clone(&active_worker.cancel_requested);
        let mut file_watch_worker = Some(active_worker);
        let mut retired_file_watch_workers = vec![retired_worker];

        assert!(!retire_file_watch_worker(
            &mut file_watch_worker,
            &mut retired_file_watch_workers,
        ));

        assert!(file_watch_worker.is_some());
        assert!(retired_cancel_requested.load(Ordering::Relaxed));
        assert!(active_cancel_requested.load(Ordering::Relaxed));
        assert_eq!(retired_file_watch_workers.len(), 1);
        assert_eq!(retired_file_watch_workers[0].generation, 1);
        assert!(retired_file_watch_workers[0].handle.is_some());

        let _ = active_release.send(());
        let Some(active_worker) = file_watch_worker.take() else {
            panic!("active file watch worker should remain joinable");
        };
        join_file_watch_worker(active_worker);

        let _ = retired_release.send(());
        while retired_file_watch_workers
            .iter()
            .any(|worker| !worker.is_finished())
        {
            thread::yield_now();
        }
        reap_finished_file_watch_workers(&mut retired_file_watch_workers);

        assert!(retired_file_watch_workers.is_empty());
        Ok(())
    }
}
