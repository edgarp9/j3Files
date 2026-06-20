use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::io;
use std::path::{Component, Path, PathBuf, Prefix};

const ERROR_CANCELLED_CODE: u32 = 1223;
const ERROR_ELEVATION_REQUIRED_CODE: u32 = 740;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellOperation {
    Open,
    OpenWith,
    ShowProperties,
    ShowContextMenu,
    Copy,
    Move,
    DeleteToRecycleBin,
    DeletePermanently,
    Rename,
    CreateFolder,
}

impl fmt::Display for ShellOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Open => "open",
            Self::OpenWith => "open with",
            Self::ShowProperties => "show properties",
            Self::ShowContextMenu => "show context menu",
            Self::Copy => "copy",
            Self::Move => "move",
            Self::DeleteToRecycleBin => "delete to recycle bin",
            Self::DeletePermanently => "delete permanently",
            Self::Rename => "rename",
            Self::CreateFolder => "create folder",
        };
        formatter.write_str(label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileNameErrorKind {
    Empty,
    HasPathSeparator,
    HasInvalidCharacter,
    HasControlCharacter,
    ReservedName,
    EndsWithSpaceOrPeriod,
}

impl fmt::Display for FileNameErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Empty => "empty",
            Self::HasPathSeparator => "contains path separator",
            Self::HasInvalidCharacter => "contains invalid Windows file name character",
            Self::HasControlCharacter => "contains control character",
            Self::ReservedName => "reserved Windows device name",
            Self::EndsWithSpaceOrPeriod => "ends with space or period",
        };
        formatter.write_str(label)
    }
}

#[derive(Debug)]
pub enum ExplorerError {
    InvalidInput {
        message: String,
    },
    InvalidFileName {
        name: OsString,
        reason: FileNameErrorKind,
    },
    InvalidLocation {
        location: PathBuf,
        reason: String,
    },
    Io {
        operation: &'static str,
        path: Option<PathBuf>,
        source: io::Error,
    },
    WindowsApi {
        operation: &'static str,
        api: &'static str,
        code: u32,
        hresult: Option<i32>,
        path: Option<PathBuf>,
        target_folder: Option<PathBuf>,
        cancelled: bool,
        elevation_required: bool,
    },
    ShellOperationFailed {
        operation: ShellOperation,
        api: &'static str,
        code: Option<u32>,
        hresult: Option<i32>,
        targets: Vec<PathBuf>,
        target_folder: Option<PathBuf>,
        cancelled: bool,
        elevation_required: bool,
    },
    Unsupported {
        operation: &'static str,
        reason: String,
    },
    StateConflict {
        message: String,
    },
    Cancelled {
        operation: &'static str,
    },
}

impl ExplorerError {
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::InvalidInput {
            message: message.into(),
        }
    }

    pub fn invalid_file_name(name: impl Into<OsString>, reason: FileNameErrorKind) -> Self {
        Self::InvalidFileName {
            name: name.into(),
            reason,
        }
    }

    pub fn invalid_location(location: impl Into<PathBuf>, reason: impl Into<String>) -> Self {
        Self::InvalidLocation {
            location: location.into(),
            reason: reason.into(),
        }
    }

    pub fn io(operation: &'static str, path: Option<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            operation,
            path,
            source,
        }
    }

    pub fn windows_api(
        operation: &'static str,
        api: &'static str,
        code: u32,
        path: Option<PathBuf>,
    ) -> Self {
        Self::windows_api_with_hresult(operation, api, code, None, path)
    }

    pub fn windows_hresult(
        operation: &'static str,
        api: &'static str,
        hresult: i32,
        path: Option<PathBuf>,
    ) -> Self {
        Self::windows_api_with_hresult(
            operation,
            api,
            windows_code_from_hresult(hresult),
            Some(hresult),
            path,
        )
    }

    fn windows_api_with_hresult(
        operation: &'static str,
        api: &'static str,
        code: u32,
        hresult: Option<i32>,
        path: Option<PathBuf>,
    ) -> Self {
        let target_folder = path.as_deref().and_then(target_folder_from_path);
        let cancelled = code == ERROR_CANCELLED_CODE;
        let elevation_required = code == ERROR_ELEVATION_REQUIRED_CODE;
        Self::WindowsApi {
            operation,
            api,
            code,
            hresult,
            path,
            target_folder,
            cancelled,
            elevation_required,
        }
    }

    pub fn shell_operation_failed(
        operation: ShellOperation,
        api: &'static str,
        code: Option<u32>,
        hresult: Option<i32>,
        targets: Vec<PathBuf>,
    ) -> Self {
        Self::shell_operation_failed_with_context(
            operation, api, code, hresult, targets, false, false,
        )
    }

    pub fn shell_operation_failed_with_context(
        operation: ShellOperation,
        api: &'static str,
        code: Option<u32>,
        hresult: Option<i32>,
        targets: Vec<PathBuf>,
        cancelled: bool,
        elevation_required: bool,
    ) -> Self {
        Self::ShellOperationFailed {
            operation,
            api,
            code,
            hresult,
            target_folder: target_folder_for_shell_operation(operation, &targets),
            targets,
            cancelled,
            elevation_required,
        }
    }

    pub fn unsupported(operation: &'static str, reason: impl Into<String>) -> Self {
        Self::Unsupported {
            operation,
            reason: reason.into(),
        }
    }

    pub fn state_conflict(message: impl Into<String>) -> Self {
        Self::StateConflict {
            message: message.into(),
        }
    }

    pub fn is_cancelled(&self) -> bool {
        match self {
            Self::WindowsApi { cancelled, .. } | Self::ShellOperationFailed { cancelled, .. } => {
                *cancelled
            }
            Self::Cancelled { .. } => true,
            _ => false,
        }
    }

    pub fn requires_elevation(&self) -> bool {
        match self {
            Self::WindowsApi {
                elevation_required, ..
            }
            | Self::ShellOperationFailed {
                elevation_required, ..
            } => *elevation_required,
            _ => false,
        }
    }

    pub fn not_found_target_paths(&self) -> Vec<&Path> {
        match self {
            Self::Io {
                path: Some(path),
                source,
                ..
            } if source.kind() == io::ErrorKind::NotFound => vec![path.as_path()],
            Self::WindowsApi {
                code,
                path: Some(path),
                ..
            } if is_not_found_code(*code) => vec![path.as_path()],
            Self::ShellOperationFailed {
                operation,
                code: Some(code),
                targets,
                ..
            } if is_not_found_code(*code)
                && shell_operation_targets_are_direct_items(*operation) =>
            {
                targets.iter().map(PathBuf::as_path).collect()
            }
            _ => Vec::new(),
        }
    }

    pub fn user_message(&self) -> String {
        explorer_error_user_message(self)
    }
}

fn explorer_error_user_message(error: &ExplorerError) -> String {
    match error {
        ExplorerError::InvalidInput { message } => message.clone(),
        ExplorerError::InvalidFileName { reason, .. } => invalid_file_name_user_message(*reason),
        ExplorerError::InvalidLocation { .. } => {
            "위치를 열 수 없습니다. 경로가 올바른지 확인해 주세요.".to_string()
        }
        ExplorerError::Io { source, .. } => io_user_message(source),
        ExplorerError::WindowsApi {
            code,
            path,
            cancelled,
            elevation_required,
            ..
        } => windows_api_failure_user_message(
            *code,
            path.as_deref(),
            PlatformFailureState {
                cancelled: *cancelled,
                elevation_required: *elevation_required,
            },
        ),
        ExplorerError::ShellOperationFailed {
            operation,
            code,
            targets,
            target_folder,
            cancelled,
            elevation_required,
            ..
        } => shell_operation_failure_user_message(
            *operation,
            *code,
            targets,
            target_folder.as_deref(),
            PlatformFailureState {
                cancelled: *cancelled,
                elevation_required: *elevation_required,
            },
        ),
        ExplorerError::Unsupported { .. } => "아직 지원하지 않는 작업입니다.".to_string(),
        ExplorerError::StateConflict { .. } => {
            "애플리케이션 상태가 요청한 작업과 맞지 않습니다.".to_string()
        }
        ExplorerError::Cancelled { .. } => "작업이 취소되었습니다.".to_string(),
    }
}

fn invalid_file_name_user_message(reason: FileNameErrorKind) -> String {
    match reason {
        FileNameErrorKind::Empty => "새 폴더 이름이 비어 있습니다.".to_string(),
        FileNameErrorKind::HasPathSeparator => {
            "폴더 이름에는 경로 구분자를 사용할 수 없습니다.".to_string()
        }
        FileNameErrorKind::HasInvalidCharacter
        | FileNameErrorKind::HasControlCharacter
        | FileNameErrorKind::ReservedName
        | FileNameErrorKind::EndsWithSpaceOrPeriod => {
            "폴더 이름이 올바르지 않습니다. 다른 이름을 입력해 주세요.".to_string()
        }
    }
}

fn io_user_message(source: &io::Error) -> String {
    match source.kind() {
        io::ErrorKind::NotFound => "위치를 찾을 수 없습니다.".to_string(),
        io::ErrorKind::PermissionDenied => "권한이 없어 작업을 완료할 수 없습니다.".to_string(),
        io::ErrorKind::AlreadyExists => "같은 이름의 파일 또는 폴더가 이미 있습니다.".to_string(),
        io::ErrorKind::InvalidInput => {
            "폴더 이름이 올바르지 않습니다. 다른 이름을 입력해 주세요.".to_string()
        }
        _ => "파일 시스템 작업을 완료할 수 없습니다.".to_string(),
    }
}

#[derive(Debug, Clone, Copy)]
struct PlatformFailureState {
    cancelled: bool,
    elevation_required: bool,
}

impl PlatformFailureState {
    fn user_message(self) -> Option<String> {
        if self.cancelled {
            Some("작업이 취소되었습니다.".to_string())
        } else if self.elevation_required {
            Some("관리자 권한이 필요해 작업을 완료할 수 없습니다.".to_string())
        } else {
            None
        }
    }
}

fn windows_api_failure_user_message(
    code: u32,
    path: Option<&Path>,
    state: PlatformFailureState,
) -> String {
    if let Some(message) = state.user_message() {
        message
    } else {
        windows_api_user_message(code, path)
    }
}

fn shell_operation_failure_user_message(
    operation: ShellOperation,
    code: Option<u32>,
    targets: &[PathBuf],
    target_folder: Option<&Path>,
    state: PlatformFailureState,
) -> String {
    if let Some(message) = state.user_message() {
        return message;
    }

    if let Some(message) = shell_operation_code_user_message(code, targets, target_folder) {
        return message;
    }

    shell_operation_fallback_user_message(operation)
}

fn shell_operation_fallback_user_message(operation: ShellOperation) -> String {
    match operation {
        ShellOperation::Open => "파일을 열 수 없습니다.".to_string(),
        ShellOperation::OpenWith => "연결 프로그램 선택 창을 열 수 없습니다.".to_string(),
        ShellOperation::ShowProperties => "속성 창을 열 수 없습니다.".to_string(),
        ShellOperation::ShowContextMenu => "컨텍스트 메뉴를 열거나 실행할 수 없습니다.".to_string(),
        ShellOperation::Copy => {
            "파일을 복사할 수 없습니다. 권한 또는 대상 위치를 확인해 주세요.".to_string()
        }
        ShellOperation::Move => {
            "파일을 이동할 수 없습니다. 권한 또는 대상 위치를 확인해 주세요.".to_string()
        }
        ShellOperation::DeleteToRecycleBin => {
            "파일을 삭제할 수 없습니다. 권한이 없거나 파일이 사용 중일 수 있습니다.".to_string()
        }
        ShellOperation::DeletePermanently => {
            "파일을 완전히 삭제할 수 없습니다. 권한이 없거나 파일이 사용 중일 수 있습니다."
                .to_string()
        }
        ShellOperation::Rename => {
            "이름을 변경할 수 없습니다. 권한 또는 이름 충돌을 확인해 주세요.".to_string()
        }
        ShellOperation::CreateFolder => {
            "새 폴더를 만들 수 없습니다. 권한 또는 이름 충돌을 확인해 주세요.".to_string()
        }
    }
}

fn shell_operation_targets_are_direct_items(operation: ShellOperation) -> bool {
    matches!(
        operation,
        ShellOperation::Open
            | ShellOperation::OpenWith
            | ShellOperation::ShowProperties
            | ShellOperation::ShowContextMenu
            | ShellOperation::DeleteToRecycleBin
            | ShellOperation::DeletePermanently
            | ShellOperation::Rename
    )
}

fn target_folder_from_path(path: &Path) -> Option<PathBuf> {
    path.parent().map(Path::to_path_buf)
}

fn target_folder_for_shell_operation(
    operation: ShellOperation,
    targets: &[PathBuf],
) -> Option<PathBuf> {
    match operation {
        ShellOperation::Copy | ShellOperation::Move => targets.last().cloned(),
        ShellOperation::Open
        | ShellOperation::OpenWith
        | ShellOperation::ShowProperties
        | ShellOperation::ShowContextMenu
        | ShellOperation::DeleteToRecycleBin
        | ShellOperation::DeletePermanently
        | ShellOperation::Rename
        | ShellOperation::CreateFolder => targets
            .first()
            .and_then(|target| target.parent().map(Path::to_path_buf)),
    }
}

#[derive(Debug, Clone, Copy)]
struct WindowsApiCode(u32);

impl WindowsApiCode {
    const HRESULT_FROM_WIN32_MASK: u32 = 0x8007_0000;

    fn from_hresult(hresult: i32) -> Self {
        let raw = hresult as u32;
        if raw & 0xffff_0000 == Self::HRESULT_FROM_WIN32_MASK {
            Self(raw & 0x0000_ffff)
        } else {
            Self(raw)
        }
    }

    fn value(self) -> u32 {
        self.0
    }

    fn user_message(self, path: Option<&Path>) -> String {
        if self.is_already_exists() {
            return "같은 이름의 파일 또는 폴더가 이미 있습니다.".to_string();
        }

        if self.is_path_too_long() {
            return "경로가 너무 길어 작업을 완료할 수 없습니다.".to_string();
        }

        if self.is_invalid_name() {
            return "폴더 이름이 올바르지 않습니다. 다른 이름을 입력해 주세요.".to_string();
        }

        if self.is_sharing_violation() {
            return "파일 또는 폴더가 사용 중이라 작업을 완료할 수 없습니다.".to_string();
        }

        if self.is_access_denied() {
            return "권한이 없어 작업을 완료할 수 없습니다.".to_string();
        }

        if self.is_network_failure() {
            return "네트워크 위치에 연결할 수 없습니다. 서버 이름, 공유 이름 또는 네트워크 연결을 확인해 주세요."
                .to_string();
        }

        if self.is_not_found() {
            return "위치를 찾을 수 없습니다.".to_string();
        }

        if path.is_some_and(is_unc_path) {
            return "네트워크 위치를 열 수 없습니다.".to_string();
        }

        "Windows에서 요청한 작업을 완료하지 못했습니다.".to_string()
    }

    fn is_known_user_actionable(self) -> bool {
        self.is_already_exists()
            || self.is_path_too_long()
            || self.is_invalid_name()
            || self.is_sharing_violation()
            || self.is_access_denied()
            || self.is_network_failure()
            || self.is_not_found()
    }

    fn is_not_found(self) -> bool {
        matches!(self.0, 2 | 3 | 15)
    }

    fn is_already_exists(self) -> bool {
        matches!(self.0, 80 | 183)
    }

    fn is_invalid_name(self) -> bool {
        matches!(self.0, 123 | 267)
    }

    fn is_path_too_long(self) -> bool {
        self.0 == 206
    }

    fn is_sharing_violation(self) -> bool {
        matches!(self.0, 32 | 33)
    }

    fn is_access_denied(self) -> bool {
        matches!(self.0, 5 | 32 | 33)
    }

    fn is_network_failure(self) -> bool {
        matches!(
            self.0,
            53 | 54 | 55 | 58 | 59 | 64 | 65 | 67 | 1219 | 1222 | 1326
        )
    }
}

fn windows_code_from_hresult(hresult: i32) -> u32 {
    WindowsApiCode::from_hresult(hresult).value()
}

fn windows_api_user_message(code: u32, path: Option<&Path>) -> String {
    WindowsApiCode(code).user_message(path)
}

fn shell_operation_code_user_message(
    code: Option<u32>,
    targets: &[PathBuf],
    target_folder: Option<&Path>,
) -> Option<String> {
    let code = WindowsApiCode(code?);
    if code.is_sharing_violation() {
        return Some("파일 또는 폴더가 사용 중이라 작업을 완료할 수 없습니다.".to_string());
    }
    if code.is_known_user_actionable() {
        let path = target_folder.or_else(|| targets.first().map(PathBuf::as_path));
        Some(code.user_message(path))
    } else {
        None
    }
}

fn is_not_found_code(code: u32) -> bool {
    WindowsApiCode(code).is_not_found()
}

fn is_unc_path(path: &Path) -> bool {
    let mut components = path.components();
    matches!(
        components.next(),
        Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::UNC(_, _) | Prefix::VerbatimUNC(_, _))
    )
}

fn write_explorer_error_diagnostic(
    error: &ExplorerError,
    formatter: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    match error {
        ExplorerError::InvalidInput { message } => write!(formatter, "invalid input: {message}"),
        ExplorerError::InvalidFileName { name, reason } => {
            write!(formatter, "invalid file name {:?}: {reason}", name)
        }
        ExplorerError::InvalidLocation { location, reason } => {
            write!(formatter, "invalid location {:?}: {reason}", location)
        }
        ExplorerError::Io {
            operation,
            path,
            source,
        } => write!(
            formatter,
            "io error during {operation} on {:?}: {source}",
            path
        ),
        ExplorerError::WindowsApi {
            operation,
            api,
            code,
            hresult,
            path,
            target_folder,
            cancelled,
            elevation_required,
        } => write_windows_api_diagnostic(
            formatter,
            WindowsApiDiagnostic {
                operation,
                api,
                code: *code,
                hresult: *hresult,
                path,
                target_folder,
                cancelled: *cancelled,
                elevation_required: *elevation_required,
            },
        ),
        ExplorerError::ShellOperationFailed {
            operation,
            api,
            code,
            hresult,
            targets,
            target_folder,
            cancelled,
            elevation_required,
        } => write_shell_operation_diagnostic(
            formatter,
            ShellOperationDiagnostic {
                operation: *operation,
                api,
                code: *code,
                hresult: *hresult,
                targets,
                target_folder,
                cancelled: *cancelled,
                elevation_required: *elevation_required,
            },
        ),
        ExplorerError::Unsupported { operation, reason } => {
            write!(formatter, "unsupported operation {operation}: {reason}")
        }
        ExplorerError::StateConflict { message } => write!(formatter, "state conflict: {message}"),
        ExplorerError::Cancelled { operation } => {
            write!(formatter, "operation cancelled: {operation}")
        }
    }
}

struct WindowsApiDiagnostic<'a> {
    operation: &'a str,
    api: &'a str,
    code: u32,
    hresult: Option<i32>,
    path: &'a Option<PathBuf>,
    target_folder: &'a Option<PathBuf>,
    cancelled: bool,
    elevation_required: bool,
}

fn write_windows_api_diagnostic(
    formatter: &mut fmt::Formatter<'_>,
    diagnostic: WindowsApiDiagnostic<'_>,
) -> fmt::Result {
    write!(
        formatter,
        "windows api error during {} via {}; code={}; hresult={:?}; target_path={:?}; target_folder={:?}; cancelled={}; elevation_required={}",
        diagnostic.operation,
        diagnostic.api,
        diagnostic.code,
        diagnostic.hresult,
        diagnostic.path,
        diagnostic.target_folder,
        diagnostic.cancelled,
        diagnostic.elevation_required
    )
}

struct ShellOperationDiagnostic<'a> {
    operation: ShellOperation,
    api: &'a str,
    code: Option<u32>,
    hresult: Option<i32>,
    targets: &'a [PathBuf],
    target_folder: &'a Option<PathBuf>,
    cancelled: bool,
    elevation_required: bool,
}

fn write_shell_operation_diagnostic(
    formatter: &mut fmt::Formatter<'_>,
    diagnostic: ShellOperationDiagnostic<'_>,
) -> fmt::Result {
    write!(
        formatter,
        "shell operation {} failed via {}; code={:?}; hresult={:?}; targets={:?}; target_folder={:?}; cancelled={}; elevation_required={}",
        diagnostic.operation,
        diagnostic.api,
        diagnostic.code,
        diagnostic.hresult,
        diagnostic.targets,
        diagnostic.target_folder,
        diagnostic.cancelled,
        diagnostic.elevation_required
    )
}

impl fmt::Display for ExplorerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_explorer_error_diagnostic(self, formatter)
    }
}

impl Error for ExplorerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_api_not_found_has_location_message() {
        let error = ExplorerError::windows_api(
            "read file attributes",
            "GetFileAttributesW",
            3,
            Some(PathBuf::from(r"C:\missing")),
        );

        assert_eq!(error.user_message(), "위치를 찾을 수 없습니다.");
    }

    #[test]
    fn windows_api_not_found_reports_target_path() {
        let path = PathBuf::from(r"C:\missing.txt");
        let error = ExplorerError::windows_api(
            "read file attributes",
            "GetFileAttributesW",
            2,
            Some(path.clone()),
        );

        assert_eq!(error.not_found_target_paths(), vec![path.as_path()]);
    }

    #[test]
    fn shell_direct_item_not_found_reports_target_path() {
        let path = PathBuf::from(r"C:\root\missing.txt");
        let error = ExplorerError::shell_operation_failed_with_context(
            ShellOperation::Open,
            "ShellExecuteExW",
            Some(2),
            None,
            vec![path.clone()],
            false,
            false,
        );

        assert_eq!(error.not_found_target_paths(), vec![path.as_path()]);
    }

    #[test]
    fn shell_transfer_not_found_does_not_report_ambiguous_targets() {
        let source = PathBuf::from(r"C:\root\missing.txt");
        let destination = PathBuf::from(r"D:\target");
        let error = ExplorerError::shell_operation_failed_with_context(
            ShellOperation::Move,
            "IFileOperation::PerformOperations",
            Some(2),
            None,
            vec![source, destination],
            false,
            false,
        );

        assert!(error.not_found_target_paths().is_empty());
    }

    #[test]
    fn windows_api_access_denied_has_permission_message() {
        let error = ExplorerError::windows_api(
            "read file attributes",
            "GetFileAttributesW",
            5,
            Some(PathBuf::from(r"C:\protected")),
        );

        assert_eq!(
            error.user_message(),
            "권한이 없어 작업을 완료할 수 없습니다."
        );
    }

    #[test]
    fn create_directory_already_exists_has_collision_message() {
        let error = ExplorerError::windows_api(
            "create folder",
            "CreateDirectoryW",
            183,
            Some(PathBuf::from(r"C:\root\New Folder")),
        );

        assert_eq!(
            error.user_message(),
            "같은 이름의 파일 또는 폴더가 이미 있습니다."
        );
    }

    #[test]
    fn create_directory_invalid_name_has_folder_name_message() {
        let error = ExplorerError::windows_api(
            "create folder",
            "CreateDirectoryW",
            123,
            Some(PathBuf::from(r"C:\root\bad:name")),
        );

        assert_eq!(
            error.user_message(),
            "폴더 이름이 올바르지 않습니다. 다른 이름을 입력해 주세요."
        );
    }

    #[test]
    fn windows_api_path_too_long_has_long_path_message() {
        let error = ExplorerError::windows_api(
            "copy file",
            "IFileOperation::PerformOperations",
            206,
            Some(PathBuf::from(r"C:\root\very-long-path")),
        );

        assert_eq!(
            error.user_message(),
            "경로가 너무 길어 작업을 완료할 수 없습니다."
        );
    }

    #[test]
    fn windows_api_sharing_violation_has_in_use_message() {
        let error = ExplorerError::windows_api(
            "move file",
            "IFileOperation::PerformOperations",
            32,
            Some(PathBuf::from(r"C:\root\locked.txt")),
        );

        assert_eq!(
            error.user_message(),
            "파일 또는 폴더가 사용 중이라 작업을 완료할 수 없습니다."
        );
    }

    #[test]
    fn invalid_file_name_keeps_internal_reason_out_of_user_message() {
        let error =
            ExplorerError::invalid_file_name("bad:name", FileNameErrorKind::HasInvalidCharacter);

        assert_eq!(
            error.user_message(),
            "폴더 이름이 올바르지 않습니다. 다른 이름을 입력해 주세요."
        );
        assert!(error.to_string().contains("bad:name"));
        assert!(error
            .to_string()
            .contains("contains invalid Windows file name character"));
    }

    #[test]
    fn windows_api_network_failure_has_network_message() {
        let error = ExplorerError::windows_api(
            "read file attributes",
            "GetFileAttributesW",
            53,
            Some(PathBuf::from(r"\\server\share")),
        );

        assert_eq!(
            error.user_message(),
            "네트워크 위치에 연결할 수 없습니다. 서버 이름, 공유 이름 또는 네트워크 연결을 확인해 주세요."
        );
    }

    #[test]
    fn shell_open_with_failure_has_user_message_without_shell_details() {
        let error = ExplorerError::shell_operation_failed(
            ShellOperation::OpenWith,
            "ShellExecuteExW",
            Some(31),
            None,
            vec![PathBuf::from(r"C:\root\readme.txt")],
        );

        assert_eq!(
            error.user_message(),
            "연결 프로그램 선택 창을 열 수 없습니다."
        );
        assert!(error.to_string().contains("ShellExecuteExW"));
        assert!(error.to_string().contains("code=Some(31)"));
    }

    #[test]
    fn windows_hresult_preserves_internal_context_and_maps_elevation() {
        let hresult = hresult_from_win32(ERROR_ELEVATION_REQUIRED_CODE);
        let error = ExplorerError::windows_hresult(
            "read known folder path",
            "SHGetKnownFolderPath",
            hresult,
            Some(PathBuf::from(r"C:\protected\file.txt")),
        );

        assert!(error.requires_elevation());
        assert!(!error.is_cancelled());
        assert_eq!(
            error.user_message(),
            "관리자 권한이 필요해 작업을 완료할 수 없습니다."
        );

        match &error {
            ExplorerError::WindowsApi {
                api,
                code,
                hresult: stored_hresult,
                path,
                target_folder,
                elevation_required,
                ..
            } => {
                assert_eq!(*api, "SHGetKnownFolderPath");
                assert_eq!(*code, ERROR_ELEVATION_REQUIRED_CODE);
                assert_eq!(*stored_hresult, Some(hresult));
                assert_eq!(path.as_deref(), Some(Path::new(r"C:\protected\file.txt")));
                assert_eq!(target_folder.as_deref(), Some(Path::new(r"C:\protected")));
                assert!(*elevation_required);
            }
            other => panic!("expected WindowsApi error, got {other:?}"),
        }

        let diagnostic = error.to_string();
        assert!(diagnostic.contains("SHGetKnownFolderPath"));
        assert!(diagnostic.contains("hresult=Some"));
        assert!(diagnostic.contains("target_folder=Some"));
        assert!(diagnostic.contains("elevation_required=true"));
    }

    #[test]
    fn shell_cancelled_error_is_distinct_from_failure_message() {
        let hresult = hresult_from_win32(ERROR_CANCELLED_CODE);
        let error = ExplorerError::shell_operation_failed_with_context(
            ShellOperation::DeleteToRecycleBin,
            "IFileOperation::PerformOperations",
            Some(ERROR_CANCELLED_CODE),
            Some(hresult),
            vec![PathBuf::from(r"C:\root\old.txt")],
            true,
            false,
        );

        assert!(error.is_cancelled());
        assert!(!error.requires_elevation());

        let message = error.user_message();
        assert_eq!(message, "작업이 취소되었습니다.");
        assert!(!message.contains("IFileOperation"));
        assert!(!message.contains(&ERROR_CANCELLED_CODE.to_string()));

        let diagnostic = error.to_string();
        assert!(diagnostic.contains("IFileOperation::PerformOperations"));
        assert!(diagnostic.contains("code=Some(1223)"));
        assert!(diagnostic.contains("cancelled=true"));
        assert!(diagnostic.contains("target_folder=Some"));
    }

    #[test]
    fn shell_collision_code_has_collision_user_message() {
        let error = ExplorerError::shell_operation_failed_with_context(
            ShellOperation::Rename,
            "IFileOperation::PerformOperations",
            Some(183),
            Some(hresult_from_win32(183)),
            vec![PathBuf::from(r"C:\root\old.txt")],
            false,
            false,
        );

        assert_eq!(
            error.user_message(),
            "같은 이름의 파일 또는 폴더가 이미 있습니다."
        );
        assert!(error.to_string().contains("hresult=Some"));
    }

    #[test]
    fn shell_access_denied_code_has_permission_user_message() {
        let error = ExplorerError::shell_operation_failed_with_context(
            ShellOperation::Copy,
            "IFileOperation::PerformOperations",
            Some(5),
            Some(hresult_from_win32(5)),
            vec![
                PathBuf::from(r"C:\root\source.txt"),
                PathBuf::from(r"C:\protected"),
            ],
            false,
            false,
        );

        assert_eq!(
            error.user_message(),
            "권한이 없어 작업을 완료할 수 없습니다."
        );
        assert!(error.to_string().contains("target_folder=Some"));
    }

    #[test]
    fn shell_sharing_violation_code_has_in_use_user_message() {
        let error = ExplorerError::shell_operation_failed_with_context(
            ShellOperation::Move,
            "IFileOperation::PerformOperations",
            Some(32),
            Some(hresult_from_win32(32)),
            vec![
                PathBuf::from(r"C:\root\locked.txt"),
                PathBuf::from(r"D:\drop"),
            ],
            false,
            false,
        );

        assert_eq!(
            error.user_message(),
            "파일 또는 폴더가 사용 중이라 작업을 완료할 수 없습니다."
        );
        assert!(error.to_string().contains("code=Some(32)"));
    }

    #[test]
    fn windows_api_user_message_hides_api_and_raw_code() {
        let error = ExplorerError::windows_api(
            "read directory",
            "FindFirstFileW",
            53,
            Some(PathBuf::from(r"\\server\share")),
        );

        let message = error.user_message();
        assert!(!message.contains("FindFirstFileW"));
        assert!(!message.contains("53"));
        assert!(message.contains("네트워크 위치"));
    }

    #[test]
    fn verbatim_unc_path_has_network_message_without_raw_details() {
        let error = ExplorerError::windows_api(
            "read file attributes",
            "GetFileAttributesW",
            9999,
            Some(PathBuf::from(r"\\?\UNC\server\share")),
        );

        let message = error.user_message();
        assert_eq!(message, "네트워크 위치를 열 수 없습니다.");
        assert!(!message.contains("GetFileAttributesW"));
        assert!(!message.contains("9999"));
    }

    fn hresult_from_win32(code: u32) -> i32 {
        ((code & 0x0000_ffff) | 0x8007_0000) as i32
    }
}
