# Release Notes

## 2026-05-18 Drag and Drop Release Candidate

### 사용자 관점 변경 사항

- j3Files 일반 폴더 목록의 선택 항목을 같은 목록의 폴더, 왼쪽 폴더 트리, Windows Explorer, Desktop으로 끌어 놓을 수 있다.
- Windows Explorer/Desktop에서 끌어 온 파일과 폴더를 j3Files 파일 목록 빈 영역 또는 왼쪽 폴더 트리 항목으로 놓을 수 있다.
- `Ctrl`은 복사, `Shift`는 이동으로 동작한다. 앱 내부 드롭에서 수정 키가 없으면 같은 드라이브 또는 같은 UNC 공유는 이동, 다른 드라이브 또는 다른 UNC 공유는 복사를 기본값으로 사용한다.
- j3Files에서 앱 밖으로 끌어낸 경우 실제 복사/이동과 기본 효과는 Explorer/Desktop 같은 드롭 대상 Shell이 결정한다.
- 복사/이동은 Windows Shell `IFileOperation`을 사용하므로 Shell 진행률, 충돌 해결, 권한 상승, 취소 동작을 따른다.

### 알려진 제한 사항

- 검색 결과에서는 j3Files 내부 드래그와 앱 밖으로 끌어내기를 지원하지 않는다. 검색 결과 표시 중 외부 파일 드롭은 파일 목록 빈 영역에서만 허용하며, 대상은 현재 탭의 실제 현재 폴더이다.
- 파일 항목 행, 검색 결과 행, 검색 상태 행, 탭바, 주소 표시줄, 버튼 영역은 드롭 대상이 아니다.
- 앱 밖으로 끌어내기와 외부 드롭 수신은 파일 시스템 경로를 담은 `CF_HDROP` 기준이다. Shell namespace/PIDL만 제공하는 고급 Shell 소스는 후속 범위이다.
- 대량 선택은 `CF_HDROP` 항목 4,096개 이하, 항목별 UTF-16 경로 32,767 코드 유닛 이하에서만 시작 또는 수신된다.
- 권한 부족, UAC 권한 상승, 긴 경로 정책, UNC 연결, 파일 사용 중 오류는 Windows Shell 결과와 시스템 정책에 따라 사용자 메시지로 표시된다.
- 드래그 중 폴더 트리 hover 자동 확장과 파일 목록/폴더 트리 가장자리 자동 스크롤은 제공하지만, 세밀한 삽입 위치 표시는 아직 제공하지 않는다.

### 개발자 변경 이력

- 드롭 효과 결정, 같은 드라이브/UNC 공유 판단, 이동 금지, 선택 스냅샷, 완료 후 갱신 대상 계산을 `domain` 규칙과 단위 테스트로 고정했다.
- OLE `IDropTarget`은 파일 목록과 폴더 트리 드롭을 감지하고, `IDataObject`의 `CF_HDROP`, 내부 드래그 식별자, 외부 `Preferred DropEffect`를 앱 이벤트로 변환한다.
- 앱 밖 Shell drag source는 `DoDragDrop` 전에 `CF_HDROP` 항목 수, 경로 길이, `HGLOBAL` 크기 overflow를 검증하고 `Preferred DropEffect`를 강제하지 않는다.
- 파일 작업 worker는 한 번에 하나만 실행하며, 드롭 작업도 붙여넣기와 같은 Shell 복사/이동 경로와 완료 후 갱신 흐름을 사용한다.
- `IDropTarget` 등록 해제는 창 destroy 처리에서 worker 정리와 설정 저장보다 먼저 실행한다.

### 검증 상태

- 최근 릴리즈 실행 검증 기록은 `docs/win32-verification.md`의 `2026-05-18 Drag and Drop Release Execution Verification`에 정리되어 있다.
- 자동 검증은 `cargo fmt --check`, `cargo test`, ignored Shell 파일 작업 스모크, `cargo check`, `cargo clippy --all-targets --all-features -- -D warnings`, `python build_release.py --no-open` 통과로 확인했다.
- 릴리즈 빌드는 `src/main.rs`의 release 전용 `windows_subsystem = "windows"`, `app.rc`의 `icon.ico` 및 탐색 버튼 아이콘 리소스, `build.rs`의 리소스 포함 흐름을 기준으로 확인했다.
- 사용자 설정 파일은 실행 파일과 같은 디렉터리에 실행 파일 stem + `.json` 이름으로 저장하는 포터블 정책을 유지한다.
- Explorer/Desktop 양방향 OLE 드래그, UAC prompt, 권한 부족 폴더, 긴 경로 정책, UNC 공유, Shell 충돌 해결 UI, 장시간 Shell 진행률 UI는 대화형 Windows 환경에서 수동 검증해야 한다.

### 후속 백로그

- Shell interop 수동 테스트 중 자동화 가능한 부분과 반드시 수동으로 남길 항목을 분리한다.
- 폴더 트리 hover 자동 확장 튜닝과 ListView/TreeView 자동 스크롤을 검토한다.
- Shell namespace/PIDL 기반 고급 드래그 소스 지원 여부를 결정한다.
- 파일 작업 worker 큐잉, 진행 상태 표시, Shell 진행률 UI와 앱 상태 행의 연결 방식을 개선할지 검토한다.
