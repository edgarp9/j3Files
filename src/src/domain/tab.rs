use super::{
    ExplorerError, ExplorerResult, FileItem, NavigationLocation, SearchCriteria, SearchDiagnostic,
    SearchProgress, SearchRunId, SortState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchState {
    Idle,
    Running {
        run_id: SearchRunId,
        criteria: SearchCriteria,
        progress: SearchProgress,
        cancel_requested: bool,
    },
    Results {
        criteria: SearchCriteria,
        items: Vec<FileItem>,
        diagnostics: Vec<SearchDiagnostic>,
        progress: SearchProgress,
    },
    Cancelled {
        criteria: SearchCriteria,
        partial_items: Vec<FileItem>,
        diagnostics: Vec<SearchDiagnostic>,
        progress: SearchProgress,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabState {
    pub id: TabId,
    current_location: NavigationLocation,
    back_history: Vec<NavigationLocation>,
    forward_history: Vec<NavigationLocation>,
    pub sort: SortState,
    pub search: SearchState,
    pub selected_items: Vec<NavigationLocation>,
}

impl TabState {
    pub fn new(id: TabId, current_location: NavigationLocation) -> Self {
        Self {
            id,
            current_location,
            back_history: Vec::new(),
            forward_history: Vec::new(),
            sort: SortState::default(),
            search: SearchState::Idle,
            selected_items: Vec::new(),
        }
    }

    pub fn from_parts(
        id: TabId,
        current_location: NavigationLocation,
        back_history: Vec<NavigationLocation>,
        forward_history: Vec<NavigationLocation>,
        sort: SortState,
    ) -> Self {
        Self {
            id,
            current_location,
            back_history,
            forward_history,
            sort,
            search: SearchState::Idle,
            selected_items: Vec::new(),
        }
    }

    pub fn with_id(mut self, id: TabId) -> Self {
        self.id = id;
        self
    }

    pub fn current_location(&self) -> &NavigationLocation {
        &self.current_location
    }

    pub fn back_history(&self) -> &[NavigationLocation] {
        &self.back_history
    }

    pub fn forward_history(&self) -> &[NavigationLocation] {
        &self.forward_history
    }

    pub fn back_location(&self) -> ExplorerResult<&NavigationLocation> {
        self.back_history
            .last()
            .ok_or_else(|| ExplorerError::state_conflict("뒤로 이동할 탐색 기록이 없습니다."))
    }

    pub fn forward_location(&self) -> ExplorerResult<&NavigationLocation> {
        self.forward_history
            .last()
            .ok_or_else(|| ExplorerError::state_conflict("앞으로 이동할 탐색 기록이 없습니다."))
    }

    pub fn navigate_to(&mut self, location: NavigationLocation) {
        let previous = std::mem::replace(&mut self.current_location, location);
        self.back_history.push(previous);
        self.forward_history.clear();
        self.search = SearchState::Idle;
        self.selected_items.clear();
    }

    pub fn go_back(&mut self) -> ExplorerResult<&NavigationLocation> {
        let previous = self
            .back_history
            .pop()
            .ok_or_else(|| ExplorerError::state_conflict("뒤로 이동할 탐색 기록이 없습니다."))?;
        let current = std::mem::replace(&mut self.current_location, previous);
        self.forward_history.push(current);
        self.search = SearchState::Idle;
        self.selected_items.clear();
        Ok(&self.current_location)
    }

    pub fn go_forward(&mut self) -> ExplorerResult<&NavigationLocation> {
        let next = self
            .forward_history
            .pop()
            .ok_or_else(|| ExplorerError::state_conflict("앞으로 이동할 탐색 기록이 없습니다."))?;
        let current = std::mem::replace(&mut self.current_location, next);
        self.back_history.push(current);
        self.search = SearchState::Idle;
        self.selected_items.clear();
        Ok(&self.current_location)
    }

    pub fn go_up(&mut self) -> ExplorerResult<Option<&NavigationLocation>> {
        let Some(parent) = self.current_location.parent()? else {
            return Ok(None);
        };
        self.navigate_to(parent);
        Ok(Some(&self.current_location))
    }

    pub fn select_only(&mut self, location: NavigationLocation) {
        self.selected_items.clear();
        self.selected_items.push(location);
    }
}
