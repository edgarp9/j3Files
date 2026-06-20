# Win32 Verification Notes

## 2026-05-09 Size/Move DPI Smoothness

적용한 정책:

- `WM_ENTERSIZEMOVE`와 `WM_EXITSIZEMOVE`로 네이티브 크기/이동 루프 상태를 추적한다.
- 루프 중 `WM_DPICHANGED`는 즉시 DPI 메트릭, 폰트, 아이콘, 레이아웃을 갱신하지 않고 pending으로 표시한다.
- 루프 중 DPI 갱신이 pending이면 `WM_SIZE`의 중간 레이아웃 재계산을 건너뛴다.
- `WM_EXITSIZEMOVE`에서 실제 현재 창 DPI를 다시 읽고, 실제 client rect 기준으로 레이아웃을 한 번 갱신한다.

현재 코드에서 직접 대상이 없는 항목:

- render-settle timer 또는 고비용 paint cache rebuild 경로가 없다.
- `UpdateWindow`, `SetWindowPos`, `ReleaseCapture` 호출 경로가 없다.
- `WM_PAINT`에서 이미지 resampling 또는 GPU/CPU upload cache를 생성하는 경로가 없다.

검증 항목:

- size/move 상태 전이는 `src/windows_main.rs` 단위 테스트로 고정한다.
- Windows GUI smoke는 `cargo check`와 `cargo test`로 컴파일 및 단위 테스트를 확인한다.
- mixed-DPI 실제 드래그는 물리 모니터 구성이 필요한 수동 검증 항목으로 남긴다.

## 2026-05-18 Drag and Drop Shell Interop

자동 테스트로 고정한 범위:

- 내부/외부 드롭의 `Ctrl` 복사, `Shift` 이동, 기본 copy/move 결정
- 같은 드라이브, 다른 드라이브, 같은 UNC 공유, 다른 UNC 공유의 기본 효과
- move에서 대상이 원본 자신 또는 원본 하위 경로인 경우 Shell 호출 전 거부
- ListView 드래그 시작 시 선택 항목 스냅샷
- copy/move 완료 후 갱신 대상 폴더 계산
- 파일 작업 worker 실행 중 두 번째 파일 작업 거부
- `DoDragDrop` 완료 결과의 취소, 드롭 없음, 복사, 이동 구분
- OLE drag source 데이터 객체의 `CF_HDROP`와 내부 드래그 형식 노출
- 앱 밖 Shell drag source가 목적지를 모르는 상태에서 `Preferred DropEffect`를 강제하지 않는 정책
- 한글/유니코드와 공백 포함 경로의 drop 규칙
- verbatim drive/UNC prefix의 같은 root 판단
- `CF_HDROP` 생성 전 항목 수, UTF-16 경로 길이, `HGLOBAL` 크기 overflow 방어
- Shell `IFileOperation`의 취소, 권한 필요, 접근 거부, 이름 충돌, 긴 경로, 공유 위반 HRESULT/Win32 코드 매핑

수동 검증 체크리스트:

- Explorer에서 j3Files 현재 파일 목록 빈 영역으로 파일/폴더를 드롭한다.
- Explorer에서 j3Files 왼쪽 폴더 트리 항목으로 파일/폴더를 드롭한다.
- j3Files 파일 목록 선택 항목을 같은 목록의 폴더 항목으로 드롭한다.
- j3Files 파일 목록 선택 항목을 왼쪽 폴더 트리 항목으로 드롭한다.
- j3Files 일반 폴더 목록 선택 항목을 Explorer 또는 Desktop으로 끌어낸다.
- 위 각 흐름에서 `Ctrl`은 복사, `Shift`는 이동으로 표시되고 실제 작업도 같은지 확인한다.
- 수정 키 없이 같은 드라이브/같은 UNC 공유 내부 드롭은 이동, 다른 드라이브/다른 UNC 공유 내부 드롭은 복사로 표시되는지 확인한다.
- j3Files에서 Explorer/Desktop으로 끌어낼 때 수정 키가 없으면 Explorer/Desktop이 대상 위치 기준 기본 효과를 결정하는지 확인한다. 같은 드라이브는 이동, 다른 드라이브는 복사, UNC 공유는 Shell 정책과 일치해야 한다.
- 외부 Shell 드롭에서 Explorer가 제공하는 `Preferred DropEffect`가 있으면 커서 효과와 실제 작업이 같은지 확인한다.
- 드래그 중 파일 항목, 검색 결과 행, 탭바, 주소 표시줄, 버튼 영역은 드롭 불가 효과를 보이는지 확인한다.
- 검색 결과 표시 중 내부 드래그는 시작되지 않고, 외부 드롭은 파일 목록 빈 영역에서만 현재 탭의 실제 현재 위치로 들어가는지 확인한다.
- 자기 자신 또는 하위 경로로 이동하려는 드롭/붙여넣기는 Shell 진행 UI가 뜨기 전에 거부되는지 확인한다.
- 파일 작업 진행 중 새 드롭, 붙여넣기, 내부 드래그 시작을 시도하면 기존 작업 진행 중 메시지 또는 드롭 불가 피드백으로 거부되는지 확인한다.
- copy/move/delete 성공 후 현재 목록, 선택 상태, 폴더 트리, 상태 행이 깨지지 않는지 확인한다.
- Shell 작업 취소는 오류 대화상자를 띄우지 않고, 실패는 사용자 메시지와 내부 진단이 분리되는지 확인한다.
- 한글 파일명과 폴더명, 공백과 `#`, 괄호 등 특수문자를 포함한 경로를 Explorer/Desktop 양방향 드래그로 확인한다.
- 긴 경로 정책이 켜진 Windows에서 `\\?\C:\...` 또는 260자를 넘는 하위 경로를 드롭하고, 실패 시 긴 경로 사용자 메시지가 표시되는지 확인한다.
- UNC 공유 `\\server\share` 안쪽 이동과 서로 다른 공유 간 복사 기본 효과를 확인한다.
- 읽기 전용 파일, 권한 부족 폴더, 다른 프로세스가 열고 있는 파일을 대상으로 드롭했을 때 Shell 오류가 권한/긴 경로/사용 중 메시지로 분류되는지 확인한다.
- 4,096개 이하 대량 선택 드래그가 시작되고, 그보다 큰 대량 선택은 앱이 과도한 메모리 할당 없이 거부하는지 확인한다.
- 드래그 중 `Esc` 취소, 드롭 대상 밖 놓기, 드롭 거부 이후 기존 선택과 상태 행이 복구되는지 확인한다.
- 파일 작업 중 앱 종료를 요청하면 작업 완료 후 닫히고, 완료 메시지가 도착해도 중복 오류나 크래시가 없는지 확인한다.
- 창 종료 시 `IDropTarget` 등록이 해제된 뒤 worker 정리와 설정 저장이 진행되는지 디버그 로그 또는 수동 smoke로 확인한다.

## 2026-05-18 Drag and Drop Release Closeout

최종 릴리즈 잠금 기준:

- 새 기능은 추가하지 않고 blocker 결함, Shell interop 불일치, 크래시 또는 파일 손실 위험만 수정한다.
- 앱 밖 Shell drag source는 `DoDragDrop` 전에 `CF_HDROP` 항목 수, UTF-16 경로 길이, 전체 `HGLOBAL` 크기 overflow 가능성을 검증한다.
- OLE drop target이 드롭 완료 이벤트를 앱 창 메시지 큐로 전달하지 못하면 `DROPEFFECT_NONE`으로 돌려 Shell source가 성공한 copy/move로 오해하지 않게 한다.
- 파일 작업 worker 실행 중 새 드롭과 내부 드래그 시작은 거부하고, 사용자에게 보이는 파일 작업 진행/종료 대기 상태 행은 한국어로 표시한다.
- `IDropTarget` 해제는 `WM_DESTROY`에서 파일 작업 worker 정리와 설정 저장보다 먼저 수행한다.
- 릴리즈 빌드는 `src/main.rs`의 `windows_subsystem = "windows"` release 조건과 `app.rc` 아이콘 리소스 포함을 기준으로 한다.
- 설정 파일은 실행 파일과 같은 디렉터리에 실행 파일 stem + `.json` 이름으로 저장하는 포터블 배포 정책을 유지한다.

자동 검증 결과:

- `cargo fmt --check`: 통과
- `cargo test`: 통과. 라이브러리 211개, GUI 엔트리 테스트 45개, Windows 파일 시스템 통합 테스트 8개 통과. `windows_shell_file_operations`의 실제 `IFileOperation` 휴지통 스모크 1개는 기본 실행에서는 프로젝트 설정대로 ignored.
- `cargo test --test windows_shell_file_operations -- --ignored --nocapture`: 통과. 임시 폴더에서 Shell copy/move/rename/delete-to-recycle-bin 스모크를 확인했다.
- `cargo check`: 통과
- `cargo clippy --all-targets --all-features -- -D warnings`: 통과
- `python build_release.py --no-open`: 통과. 릴리즈 빌드 스크립트가 `cargo build --release`를 완료했다.

문서 대조 결과:

- 자동 테스트로 고정된 copy/move 효과 결정, 이동 금지, 선택 스냅샷, 완료 후 갱신 대상, worker 중복 거부, `CF_HDROP` 방어 항목은 아래 수동 체크리스트의 핵심 정책과 일치한다.
- 실제 Explorer/Desktop 양방향 OLE 드래그, 권한 상승/권한 부족, 긴 경로 정책, UNC 공유, Shell 충돌 해결 UI, 장시간 진행률 UI는 자동 결과만으로 완료 판정하지 않고 수동 확인 대상으로 유지한다.

최종 수동 검증 체크리스트:

- Explorer/Desktop에서 j3Files 현재 파일 목록 빈 영역으로 파일/폴더를 드롭한다.
- Explorer/Desktop에서 j3Files 왼쪽 폴더 트리 항목으로 파일/폴더를 드롭한다.
- j3Files 파일 목록 선택 항목을 같은 목록의 폴더 항목으로 드롭한다.
- j3Files 파일 목록 선택 항목을 왼쪽 폴더 트리 항목으로 드롭한다.
- j3Files 일반 폴더 목록 선택 항목을 Explorer 또는 Desktop으로 끌어낸다.
- 각 흐름에서 `Ctrl` 복사, `Shift` 이동, 수정 키 없는 기본 효과가 커서 피드백과 실제 작업에서 일치하는지 확인한다.
- 검색 결과 표시 중 파일 목록 행에서는 내부 드래그와 외부 드롭이 시작되지 않고, 외부 드롭은 빈 영역에서만 현재 탭의 실제 현재 위치로 들어가는지 확인한다.
- 파일 항목 행, 검색 상태 행, 탭바, 주소 표시줄, 버튼 영역에서는 드롭 불가 효과가 표시되는지 확인한다.
- 다중 선택, 취소, 드롭 거부, 권한 부족, 사용 중인 파일, 한글/유니코드 경로, 긴 경로, UNC 공유 경로를 확인한다.
- 파일 작업 진행 중 추가 드롭/붙여넣기/내부 드래그 시작이 거부되는지 확인한다.
- 파일 작업 중 앱 종료를 요청하면 작업 완료 후 닫히고 완료 메시지 처리 중 크래시가 없는지 확인한다.
- 릴리즈 실행 파일에서 콘솔 창이 뜨지 않고, 작업 표시줄/Alt+Tab/창 제목 표시줄 아이콘과 탐색 버튼 리소스가 표시되는지 확인한다.
- 릴리즈 실행 파일 옆에 `j3Files.json` 설정 파일이 생성/갱신되는지 확인한다.

알려진 제한 사항:

- 앱 밖으로 끌어내기는 일반 폴더 목록 선택 항목을 `CF_HDROP` 파일 목록으로 제공하는 범위이다. 검색 결과에서 앱 밖으로 끌어내기는 지원하지 않는다.
- Shell namespace/PIDL만 제공하는 고급 드래그 소스는 현재 드롭 수신 범위가 아니다.
- 드래그 중 폴더 트리 hover 자동 확장과 ListView/TreeView 가장자리 자동 스크롤은 제공하지만, 세밀한 삽입 위치 표시는 아직 제공하지 않는다.
- `CF_HDROP` 항목 수 4,096개 초과 또는 항목별 UTF-16 경로 길이 32,767 코드 유닛 초과는 드래그 시작 또는 드롭 수신 단계에서 거부된다.

이번 자동 실행에서 확인하지 못한 항목:

- 실제 Explorer/Desktop과의 양방향 OLE 드래그앤드롭은 대화형 데스크톱 조작이 필요해 자동 실행하지 못했다.
- 권한 상승 UAC prompt, 권한 부족 폴더, 장시간 Shell 진행률 UI, Shell 충돌 해결 UI는 수동 Windows 환경이 필요하다.
- 긴 경로 정책과 UNC 공유는 로컬 정책/네트워크 공유 준비가 필요해 자동 실행하지 못했다.
- 릴리즈 산출물 내부 파일 직접 열람은 `target/` 제외 규칙 때문에 수행하지 않았고, 빌드 스크립트 성공과 소스의 subsystem/resource 설정으로만 확인했다.

## 2026-05-18 Drag and Drop UX Follow-up

자동 테스트로 고정한 범위:

- hover 대상이 바뀌면 폴더 트리 자동 확장 pending 대상과 기준 시간이 갱신된다.
- hover 시간이 임계값 미만이면 자동 확장을 실행하지 않는다.
- 드래그 종료/취소에 해당하는 clear 경로에서 hover pending 상태가 제거된다.
- 컨트롤 위/아래 edge 좌표가 자동 스크롤 방향 `Up`/`Down`으로 결정되고, 중앙 영역과 잘못된 geometry는 스크롤하지 않는다.
- 기존 copy/move/none 효과 결정 테스트는 그대로 유지한다.

자동 검증 결과:

- `cargo fmt --check`: 통과
- `cargo test`: 통과. 라이브러리 217개, GUI 엔트리 테스트 45개, Windows 파일 시스템 통합 테스트 8개 통과. `windows_shell_file_operations`의 실제 `IFileOperation` 휴지통 스모크 1개는 기본 실행에서는 프로젝트 설정대로 ignored.
- `cargo check`: 통과

수동 검증 체크리스트:

- j3Files 내부 파일 목록 선택 항목을 왼쪽 폴더 트리의 접힌 폴더 위에 올려 약 700ms 후 자동 확장되는지 확인한다.
- Explorer/Desktop에서 끌어 온 파일/폴더를 왼쪽 폴더 트리의 접힌 폴더 위에 올려 자동 확장되는지 확인한다.
- 드래그 중 TreeView 위/아래 edge에 머물면 폴더 트리가 한 줄씩 자동 스크롤되는지 확인한다.
- 드래그 중 ListView 위/아래 edge에 머물면 파일 목록이 한 줄씩 자동 스크롤되는지 확인한다.
- `Esc` 취소, 드롭 불가 영역 이동, 드롭 거부, 창 종료 후 자동 확장/스크롤 timer가 남아 계속 스크롤하지 않는지 확인한다.
- `Ctrl` 복사, `Shift` 이동, 파일 항목/검색 결과/버튼 영역의 드롭 불가 커서 효과가 이전 정책과 일치하는지 확인한다.

후속 백로그:

- 파일 목록 행 사이의 세밀한 삽입 위치 표시는 현재 드롭 대상 의미가 폴더 위치로 제한되어 있어 이번 범위에서 제외한다.
- 자동 스크롤 edge 폭과 100ms line-scroll 주기는 실제 장치/마우스 감도에서 추가 튜닝할 수 있다.

## 2026-05-18 Drag and Drop Bulk Stability

자동 테스트로 고정한 범위:

- `CF_HDROP` 전체 byte size 계산의 overflow와 64 MiB 메모리 상한 거부
- 앱 밖 Shell drag source의 최대 파일 개수 4,096개 경계값
- 항목별 UTF-16 경로 길이 32,767 코드 유닛 경계값
- 한글/유니코드, verbatim UNC 경로의 `CF_HDROP` 왕복 보존
- 대소문자 표기만 다른 중복 드래그 소스 제거
- 대량 선택 검증 실패가 `DoDragDrop` 전 사용자 메시지로 거부되는 흐름
- 기존 copy/move 효과 결정, 자기 자신/하위 경로 이동 거부, worker 중복 실행 보호 유지

자동 검증 결과:

- `cargo fmt --check`: 통과
- `cargo test`: 통과. 라이브러리 222개, GUI 엔트리 테스트 45개, Windows 파일 시스템 통합 테스트 8개 통과. `windows_shell_file_operations`의 실제 `IFileOperation` 휴지통 스모크 1개는 기본 실행에서는 프로젝트 설정대로 ignored.
- `cargo check`: 통과
- `cargo clippy --all-targets --all-features -- -D warnings`: 통과

수동 검증 체크리스트:

- 수백 개 파일을 다중 선택해 Explorer/Desktop으로 끌어내고 앱이 멈추지 않는지 확인한다.
- 4,096개 이하 선택은 드래그가 시작되고, 4,096개 초과 선택은 Shell 진행 UI 없이 거부되는지 확인한다.
- 한글, 이모지, 결합 문자, 공백, 괄호, `#`을 포함한 파일/폴더명을 Explorer/Desktop 양방향 드래그로 확인한다.
- 긴 경로 정책이 켜진 Windows에서 260자를 넘는 경로와 `\\?\C:\...`, `\\?\UNC\server\share\...` 경로를 확인한다.
- UNC 공유 `\\server\share` 내부 이동과 다른 공유/드라이브로의 복사 기본 효과를 확인한다.
- 존재하지 않는 항목, 권한 부족 폴더, 사용 중인 파일이 섞였을 때 Shell 오류/취소 후 선택 상태, 상태 행, 목록 갱신이 깨지지 않는지 확인한다.
- 파일 작업 진행 중 추가 드롭/붙여넣기/내부 드래그 시작이 거부되고 기존 worker 완료 후 정상 갱신되는지 확인한다.
- 드래그 중 `Esc` 취소, 드롭 대상 밖 놓기, 드롭 불가 영역 놓기 후 hover timer와 자동 스크롤이 남지 않는지 확인한다.

후속 범위:

- Shell namespace/PIDL만 제공하는 고급 드래그 소스 수신과 가상 파일 항목 처리는 이번 범위에서 제외하고 후속 과제로 유지한다.

이번 자동 실행에서 확인하지 못한 항목:

- 실제 Explorer/Desktop과의 양방향 OLE 드래그앤드롭, 수백 개 파일 수동 드래그, 긴 경로 정책, UNC 공유, 권한 상승/UAC, Shell 충돌 해결 UI는 대화형 Windows 환경과 준비된 네트워크/정책 조건이 필요해 수동 검증으로 남긴다.

## 2026-05-18 Drag and Drop Loss Prevention

자동 테스트로 고정한 범위:

- Shell 파일 작업 취소는 사용자 오류 대화상자 표시 대상이 아니다.
- copy가 예상 대상 일부만 만든 경우 실행 취소 후보를 만들지 않는다.
- move가 예상 대상을 만들었더라도 원본 경로가 남아 있으면 실행 취소 후보와 완료 선택 항목을 만들지 않는다.
- 파일 작업 worker 실행 중 두 번째 파일 작업 시작은 거부된다.
- move/copy 완료 후 갱신 대상은 copy 대상 폴더, move 원본 부모와 대상 폴더로 계산된다.
- 공유 위반은 "사용 중" 메시지로, 권한 상승 필요/접근 거부/경로 없음은 각각 관리자 권한/권한/위치 없음 메시지로 분류된다.
- 기존 drop 효과, 긴 경로, 대량 선택, 자기 자신/하위 경로 이동 거부 테스트를 유지한다.

자동 검증 결과:

- `cargo fmt --check`: 통과
- `cargo test`: 통과. 기본 실행에서 ignored인 `windows_shell_file_operations` Shell 스모크 1개는 별도 실행 대상으로 유지된다.
- `cargo test --test windows_shell_file_operations -- --ignored --nocapture`: 통과. 임시 폴더에서 Shell copy/move/rename/delete-to-recycle-bin 스모크를 확인했다.
- `cargo check`: 통과
- `cargo clippy --all-targets --all-features -- -D warnings`: 통과

수동 검증 체크리스트:

- Shell 충돌 해결 UI에서 사용자가 `취소`를 선택했을 때 오류 대화상자가 뜨지 않고 목록과 선택 상태가 깨지지 않는지 확인한다.
- Shell 충돌 해결 UI에서 일부 항목을 건너뛰거나 취소한 뒤 `Ctrl+Z` 후보가 남지 않는지 확인한다.
- 권한 부족 폴더로 copy/move 드롭했을 때 Shell 권한 UI 또는 권한 오류 메시지가 실제 상황과 맞는지 확인한다.
- 관리자 권한이 필요한 위치로 드롭했을 때 UAC/권한 상승 흐름이 Shell UI로 표시되고, 거부 시 앱 오류 메시지가 중복 표시되지 않는지 확인한다.
- 다른 프로세스가 열고 있는 파일 이동을 시도했을 때 "사용 중" 메시지로 분류되는지 확인한다.
- 드롭 소스 또는 대상 경로가 사라진 상태에서 작업을 시도했을 때 "위치를 찾을 수 없습니다." 계열 메시지와 목록 갱신이 일치하는지 확인한다.
- 파일 작업 진행 중 추가 드롭, 내부 드래그 시작, 붙여넣기를 시도했을 때 기존 작업 진행 중 상태 행 또는 불가 피드백으로 거부되는지 확인한다.
- move 드롭 실패 또는 취소 후 원본 선택 상태가 보존되고, 현재 폴더 목록과 폴더 트리가 정상 갱신되는지 확인한다.
- 앱 종료 요청 후 파일 작업이 완료되거나 취소되어 완료 메시지가 도착해도 크래시 없이 창이 정리되는지 확인한다.

후속 백로그:

- Shell 진행률 세부 단계와 앱 내부 상태 행을 더 정교하게 동기화하는 작업은 별도 범위로 둔다.
- 실제 Explorer/Desktop OLE 상호 운용, UAC, 충돌 해결 UI, 공유 위반 재현은 자동화가 제한되어 수동 smoke로 유지한다.

## 2026-05-18 Drag and Drop Boundary Cleanup

자동 테스트로 고정한 범위:

- 외부 drop 허용 효과와 `Preferred DropEffect`를 `domain` 순수 규칙으로 변환해 copy/move 기본값을 결정한다.
- 클립보드와 OLE drag data가 공유하는 `platform::hdrop`에서 `CF_HDROP` 항목 수, UTF-16 경로 길이, 전체 `HGLOBAL` byte size overflow와 64 MiB 상한을 검증한다.
- 한글/유니코드, 공백, verbatim UNC 경로의 `CF_HDROP` 왕복 보존을 공통 모듈 테스트로 유지한다.
- 파일 작업 worker copy/move 요청은 붙여넣기 전용 타입이 아니라 `Transfer`와 `domain::DropOperation`으로 실행되며, move 자기 자신/하위 경로 거부를 Shell 호출 전에 재검증한다.
- copy/move/delete 영향 폴더 계산, worker 중복 실행 보호, 취소/부분 실패 시 실행 취소 후보 미생성 정책을 유지한다.

자동 검증 결과:

- `cargo fmt --check`: 통과
- `cargo test`: 통과. 라이브러리 228개, GUI 엔트리 테스트 50개, Windows 파일 시스템 통합 테스트 8개 통과. `windows_shell_file_operations`의 실제 `IFileOperation` 휴지통 스모크 1개는 기본 실행에서는 ignored.
- `cargo test --test windows_shell_file_operations -- --ignored --nocapture`: 통과. 임시 폴더에서 Shell create/copy/move/rename/delete-to-recycle-bin 스모크를 확인했다.
- `cargo check`: 통과
- `cargo clippy --all-targets --all-features -- -D warnings`: 통과

수동 검증 체크리스트:

- Explorer/Desktop에서 j3Files로 드롭하는 흐름과 j3Files에서 Explorer/Desktop으로 끌어내는 흐름이 기존 copy/move 커서 효과와 실제 작업 결과를 유지하는지 확인한다.
- 붙여넣기, 내부 드롭, 외부 Shell 드롭이 모두 같은 파일 작업 진행 상태 행과 완료 후 갱신 정책을 따르는지 확인한다.
- 파일 작업 진행 중 추가 붙여넣기/드롭/내부 드래그 시작이 계속 거부되는지 확인한다.
- 클립보드 파일 붙여넣기와 앱 밖 드래그에서 4,096개 초과 또는 과도한 긴 경로가 Shell 진행 UI 전에 사용자 메시지로 거부되는지 확인한다.

후속 백로그:

- Explorer/Desktop 양방향 OLE 실제 조작, UAC, Shell 충돌 해결 UI, 긴 경로 정책, UNC 공유는 자동화 한계가 있어 계속 수동 smoke로 유지한다.
- Shell namespace/PIDL만 제공하는 고급 드래그 소스와 가상 파일 항목은 현재 `CF_HDROP` 범위 밖으로 둔다.

## 2026-05-18 Drag and Drop Final Review

최종 리뷰 결과:

- 새 기능은 추가하지 않고 릴리즈 전 blocker만 점검했다.
- 외부 `Preferred DropEffect`가 copy/move 비트를 동시에 갖거나 알 수 없는 비트를 포함하는 경우 선호 효과로 인정하지 않고, 기존 domain 기본 정책이 안전한 copy fallback을 결정하도록 고정했다.
- `docs/domain.md`의 드래그앤드롭 단계 설명과 `docs/release-notes.md`의 자동 스크롤 제한 설명을 현재 구현 범위와 맞췄다.

자동 검증 결과:

- `cargo fmt --check`: 통과
- `cargo test`: 통과. 라이브러리 229개, GUI 엔트리 테스트 50개, Windows 파일 시스템 통합 테스트 8개 통과. `windows_shell_file_operations`의 실제 `IFileOperation` 휴지통 스모크 1개는 기본 실행에서는 ignored.
- `cargo test --test windows_shell_file_operations -- --ignored --nocapture`: 통과. 임시 폴더에서 Shell create/copy/move/rename/delete-to-recycle-bin 스모크를 확인했다.
- `cargo check`: 통과
- `cargo clippy --all-targets --all-features -- -D warnings`: 통과
- `python build_release.py --no-open`: 통과. 릴리즈 빌드 스크립트가 `cargo build --release`를 완료했다.

최종 수동 검증 체크리스트:

- Explorer/Desktop에서 j3Files 파일 목록 빈 영역과 왼쪽 폴더 트리 항목으로 파일/폴더를 드롭한다.
- j3Files 파일 목록 선택 항목을 같은 목록의 폴더 항목과 왼쪽 폴더 트리 항목으로 드롭한다.
- j3Files 일반 폴더 목록 선택 항목을 Explorer/Desktop으로 끌어낸다.
- 위 흐름에서 `Ctrl` 복사, `Shift` 이동, 수정 키 없는 기본 효과가 커서 피드백과 실제 작업 결과에서 일치하는지 확인한다.
- 다중 선택, `Esc` 취소, 드롭 대상 밖 놓기, 드롭 거부, 권한 부족, 사용 중인 파일, 한글/유니코드 경로, 긴 경로, UNC 공유 경로를 확인한다.
- Shell 충돌 해결 UI, 권한 상승 UI, 장시간 Shell 진행률 UI에서 취소/부분 실패 후 오류 대화상자, 실행 취소 후보, 선택 상태, 목록/폴더 트리 갱신이 깨지지 않는지 확인한다.
- 파일 작업 진행 중 추가 드롭/붙여넣기/내부 드래그 시작이 거부되고, 앱 종료 요청은 파일 작업 완료 후 창 정리로 이어지는지 확인한다.

이번 자동 실행에서 확인하지 못한 항목:

- Explorer/Desktop과의 실제 양방향 OLE 드래그앤드롭은 대화형 데스크톱 조작이 필요해 자동 실행하지 못했다.
- UAC prompt, 권한 부족 폴더, Shell 충돌 해결 UI, 장시간 Shell 진행률 UI는 수동 Windows 환경이 필요하다.
- 긴 경로 정책과 UNC 공유는 로컬 정책/네트워크 공유 준비가 필요해 자동 실행하지 못했다.

## 2026-05-18 Drag and Drop Release Execution Verification

릴리즈 후보 점검 결과:

- 새 기능은 추가하지 않고 릴리즈 실행/배포 관점의 blocker만 점검했다.
- 현재 git diff 기준으로 드래그앤드롭 구현의 raw Win32/OLE/COM 타입은 `platform` 경계에 머물고, `entry`/`app` 경계에는 `NavigationLocation`, `DropOperation`, 파일 작업 worker 요청으로 변환되어 전달된다.
- 드래그앤드롭 관련 `dbg!`, 임시 `println!`, 테스트 전용 런타임 분기는 확인되지 않았다. 남아 있는 `eprintln!`은 빌드 스크립트 또는 복구/진단용 오류 로그이며, 릴리즈 GUI는 release 전용 Windows subsystem 설정으로 별도 콘솔 창을 만들지 않는 정책을 유지한다.
- 릴리즈 실행 파일 아이콘은 `app.rc`가 `icon.ico`와 탐색 버튼 아이콘을 포함하고, `build.rs`가 해당 리소스를 빌드에 포함하는 구조로 확인했다.
- 콘솔 창 미표시 정책은 `src/main.rs`의 `#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]`로 확인했다.
- 설정 파일 위치 정책은 `NativeUserSettingsStore::new()`가 `current_exe()`의 디렉터리와 실행 파일 stem + `.json` 이름을 사용하는 구조로 확인했다.
- `target/` 제외 규칙 때문에 릴리즈 실행 파일을 직접 열람하거나 PE 리소스를 덤프하지는 않았다. 이번 확인은 릴리즈 빌드 성공과 소스의 subsystem/resource/settings 정책 대조를 기준으로 한다.

자동 검증 결과:

- `cargo fmt --check`: 통과
- `cargo test`: 통과. 라이브러리 229개, GUI 엔트리 테스트 50개, Windows 파일 시스템 통합 테스트 8개 통과. `windows_shell_file_operations`의 실제 `IFileOperation` 휴지통 스모크 1개는 기본 실행에서는 프로젝트 설정대로 ignored.
- `cargo test --test windows_shell_file_operations -- --ignored --nocapture`: 통과. 임시 폴더에서 Shell create/copy/move/rename/delete-to-recycle-bin 스모크를 확인했다.
- `cargo check`: 통과
- `cargo clippy --all-targets --all-features -- -D warnings`: 통과
- `python build_release.py --no-open`: 통과. 릴리즈 빌드 스크립트가 `cargo build --release`를 완료했다.

릴리즈 후보 수동 검증 체크리스트:

- Explorer/Desktop에서 j3Files 파일 목록 빈 영역과 왼쪽 폴더 트리 항목으로 파일/폴더를 드롭한다.
- j3Files 파일 목록 선택 항목을 같은 목록의 폴더 항목과 왼쪽 폴더 트리 항목으로 드롭한다.
- j3Files 일반 폴더 목록 선택 항목을 Explorer/Desktop으로 끌어낸다.
- 위 흐름에서 `Ctrl` 복사, `Shift` 이동, 수정 키 없는 기본 효과가 커서 피드백과 실제 작업 결과에서 일치하는지 확인한다.
- 다중 선택, `Esc` 취소, 드롭 대상 밖 놓기, 드롭 거부, 권한 부족, 사용 중인 파일, 한글/유니코드 경로, 긴 경로, UNC 공유 경로를 확인한다.
- Shell 충돌 해결 UI, 권한 상승 UI, 장시간 Shell 진행률 UI에서 취소/부분 실패 후 오류 대화상자, 실행 취소 후보, 선택 상태, 목록/폴더 트리 갱신이 깨지지 않는지 확인한다.
- 파일 작업 진행 중 추가 드롭/붙여넣기/내부 드래그 시작이 거부되고, 앱 종료 요청은 파일 작업 완료 후 창 정리로 이어지는지 확인한다.
- 릴리즈 실행 파일에서 콘솔 창이 뜨지 않고 작업 표시줄/Alt+Tab/창 제목 표시줄 아이콘이 표시되는지 확인한다.
- 릴리즈 실행 파일 옆에 실행 파일 stem + `.json` 설정 파일이 생성/갱신되는지 확인한다.

이번 자동 실행에서 확인하지 못한 항목:

- Explorer/Desktop과의 실제 양방향 OLE 드래그앤드롭은 대화형 데스크톱 조작이 필요해 자동 실행하지 못했다.
- 릴리즈 실행 파일의 실제 창 아이콘과 콘솔 창 미표시는 대화형 실행 확인이 필요하다.
- UAC prompt, 권한 부족 폴더, Shell 충돌 해결 UI, 장시간 Shell 진행률 UI는 수동 Windows 환경이 필요하다.
- 긴 경로 정책과 UNC 공유는 로컬 정책/네트워크 공유 준비가 필요해 자동 실행하지 못했다.

## 2026-06-18 Menu Execution Verification

목표:

- 상단 메뉴 구조와 주요 메뉴 명령이 실제 Win32 창에서 오류 없이 실행되는지 확인했다.
- UI 레이아웃, 검색줄 표시, 메뉴 체크 상태, 설정 저장, 종료 동작, Windows/Linux 빌드 경계를 함께 점검했다.

자동 검증 결과:

- Win32 메뉴 스모크: 통과. 임시 실행 파일과 임시 시작 폴더로 `j3Files` 창을 띄우고 `File`, `Edit`, `View`, `Go`, `Bookmarks`, `Tabs`, `Search` 상단 메뉴 구조를 확인했다.
- Win32 메뉴 명령 스모크: 통과. 54개 메뉴/가속 명령을 실제 창의 `WM_COMMAND` 경로로 실행했으며 타임아웃, 창 비정상 종료, stdout/stderr 로그가 없었다.
- `Edit > Paste` 안전 스모크: 통과. 현재 Windows 클립보드에 `CF_HDROP` 파일 목록이 없는 것을 확인한 뒤 실제 창의 `Paste` 명령을 no-op 경로로 실행했고 stdout/stderr 로그가 없었다.
- 검색 체크 동기화: 통과. `sub` 체크박스 클릭 후 `Search > Include Subfolders` 메뉴 체크가 켜지고, 메뉴 토글 후 다시 꺼지는 것을 확인했다.
- Font dialog: 통과. `View > Font...` 명령으로 표준 dialog가 열리고 닫힌 뒤 메인 창이 유지되는 것을 확인했다.
- 설정 저장: 통과. 테마, 탭 시작 설정, 북마크 변경 후 종료 시 임시 실행 파일 옆 설정 파일이 생성되는 것을 확인했다.
- UI 레이아웃 스모크: 통과. 980x620 창에서 검색줄을 표시한 뒤 TreeView, Tab, toolbar, address edit, ListView, search edit/button/checkbox/cancel 컨트롤이 모두 표시되고 양수 크기를 갖는 것을 확인했다. `PrintWindow` 캡처의 창 제목은 `j3Files`였다.
- Windows Shell 파일 작업 스모크: 통과. `windows_shell_file_operations` ignored 테스트를 명시 실행해 create/copy/move/rename/delete-to-recycle-bin 경로를 임시 폴더에서 확인했다.
- Linux 타깃 확인: 의도된 실패. `x86_64-unknown-linux-gnu`에서는 Windows-only 앱임을 단일 `compile_error!`로 보고하고 Windows 전용 import 오류가 뒤따르지 않는다.

수정한 문제:

- `sub` 체크박스 클릭은 검색 범위 상태를 바꾸지만 메뉴 체크 표시를 다시 만들지 않아 `Search > Include Subfolders` 표시가 다음 메뉴 표시 전에 오래된 상태로 남을 수 있었다. 체크박스 명령을 메뉴 동기화 경로로 연결했다.
- 북마크 추가 성공 시 사용자 설정 저장 예약이 빠져 재시작 후 북마크가 복원되지 않을 수 있었다. 새 북마크가 실제 추가된 경우에만 설정 저장을 예약하도록 했다.
- 기록이 없는 상태에서 `Go > Back` 또는 `Go > Forward`를 실행하면 domain의 상태 충돌 오류가 entry 로그로 남았다. Win32 entry 경계에서는 기록 없음 사용자 동작을 no-op으로 처리해 오류 로그를 남기지 않게 했다.
- Linux 타깃에서 Windows-only compile error 뒤에 `std::os::windows` 등 다수의 후속 컴파일 오류가 따라왔다. non-Windows에서는 crate entry/lib 경계를 조기 차단하도록 정리했다.

이번 자동 실행에서 확인하지 못한 항목:

- `Edit > Paste`의 파일 목록 포함 클립보드 실제 붙여넣기는 사용자 Windows 클립보드에 파일 잘라내기 데이터가 있을 경우 외부 파일 이동을 일으킬 수 있어 자동 메뉴 스모크에서는 수행하지 않았다. 클립보드 파싱과 파일 작업 worker 경로는 단위/통합 테스트로 확인한다.
- 실제 마우스 포인터로 모든 메뉴를 하나씩 클릭하는 화면 자동화는 Computer Use 플러그인 초기화 실패로 수행하지 못했다. 대신 실제 창의 메뉴 command dispatch, 체크박스 `BM_CLICK`, Font dialog, 설정 저장, `PrintWindow` 캡처로 보강했다.

## 2026-06-18 Menu Execution Regression Test

추가한 자동 검증:

- `tests/windows_menu_execution.rs`를 추가했다. 기본 `cargo test`에서는 ignored 상태이며, 명시 실행 시 실제 `j3Files` Win32 창을 띄우고 상단 메뉴 구조와 메뉴 command id를 열거한다.
- 테스트는 임시 시작 폴더에 파일과 폴더를 만든 뒤 실제 창의 `WM_COMMAND` 경로로 메뉴 기능을 실행한다.
- `File > New Folder`, `Edit > Select All`, 북마크 추가/동적 북마크 열기/삭제, 탭 열기/전환/닫기/복원, 검색줄 표시와 `sub` 체크박스-메뉴 체크 동기화, 테마/정렬/표시 옵션, 표준 Font dialog 열기 후 취소, known folder와 drive 메뉴 이동, 종료를 확인한다.
- 현재 클립보드에 `CF_HDROP` 파일 목록이 없으면 `Edit > Paste`도 실제 명령으로 실행한다. 파일 목록이 있으면 사용자 파일 이동/복사를 피하기 위해 해당 실행만 건너뛰고 로그를 남기도록 했다.

이번 실행 결과:

- `cargo fmt --check`: 통과
- `cargo test`: 통과. 라이브러리 291개, GUI 엔트리 테스트 106개, Windows 파일 시스템 통합 테스트 8개 통과. GUI 메뉴 스모크와 Shell 파일 작업 스모크는 ignored로 유지된다.
- `J3FILES_SMOKE_EXE=<임시 설치 exe> cargo test --test windows_menu_execution -- --ignored --nocapture`: 통과. 임시 설치 실행 파일로 실제 창을 띄워 메뉴 스모크를 확인했다.
- `cargo test --test windows_shell_file_operations -- --ignored --nocapture`: 통과. 임시 폴더에서 Shell create/copy/move/rename/delete-to-recycle-bin 경로를 확인했다.
- `cargo clippy --all-targets --all-features -- -D warnings`: 통과
- `cargo check --target x86_64-unknown-linux-gnu`: 의도된 실패. non-Windows 타깃에서 `j3Files is a Windows-only file explorer application.` 단일 compile error로 조기 차단된다.

| 메뉴 | 기능 | Windows 동작 | Linux 기존 동작 | 문제 여부 | 원인 | 수정 내용 | 재검증 결과 |
| -- | -- | ---------- | ----------- | ----- | -- | ----- | ------ |
| File | New Folder, Open, Open With, Rename, Delete, Properties, Exit | 실제 Win32 창에서 안전한 순서로 command dispatch 실행. 새 폴더는 임시 시작 폴더에만 생성하고 종료까지 확인. | Windows-only 앱이므로 non-Windows 빌드는 compile error로 중단. | 없음 | 해당 없음 | GUI 메뉴 스모크 테스트 추가 | 통과 |
| Edit | Undo, Cut, Copy, Paste, Select All | 선택 없음 no-op 경로와 임시 폴더 선택 경로를 확인. `Paste`는 `CF_HDROP`가 없을 때 실행. | 동일하게 실행 대상 아님 | 없음 | 해당 없음 | GUI 메뉴 스모크 테스트 추가 | 통과 |
| View | Refresh, Sort, Show Hidden/System, Theme, Font, Reset Font | 메뉴 체크/명령 실행, 창 크기 980x620 변경 후 주요 컨트롤 양수 크기 확인. Font dialog는 열고 취소. | 동일하게 실행 대상 아님 | 없음 | 해당 없음 | GUI 메뉴 스모크 테스트 추가 | 통과 |
| Go | Back, Forward, Up, Home/Desktop/Downloads/Documents, Drives | 기록 없음 back/forward no-op, known folder/drive 동적 메뉴 이동 명령 실행. | 동일하게 실행 대상 아님 | 없음 | 해당 없음 | GUI 메뉴 스모크 테스트 추가 | 통과 |
| Bookmarks | Add Current, Add Selected Folder, dynamic bookmark, Remove Current | 현재 위치와 선택 폴더 북마크를 추가하고 동적 메뉴 항목으로 열어 삭제. | 동일하게 실행 대상 아님 | 없음 | 해당 없음 | GUI 메뉴 스모크 테스트 추가 | 통과 |
| Tabs | New, Open Selected Folder, Close, Next, Reopen, Move Left/Right, Startup options | 탭 생성/전환/정렬/닫기/복원 및 시작 옵션 명령 실행. | 동일하게 실행 대상 아님 | 없음 | 해당 없음 | GUI 메뉴 스모크 테스트 추가 | 통과 |
| Search | Find, Include Subfolders, Cancel, Close, `sub` 체크박스 | 검색줄 표시 후 체크박스 클릭과 메뉴 토글의 체크 상태 동기화 확인. | 동일하게 실행 대상 아님 | 없음 | 해당 없음 | GUI 메뉴 스모크 테스트 추가 | 통과 |

이번 자동 실행에서 확인하지 못한 항목:

- 실제 마우스 포인터로 메뉴를 클릭하는 화면 자동화는 Windows 앱 제어 플러그인의 내부 런타임 export 오류로 수행하지 못했다. 대신 재현 가능한 Win32 창 기반 메뉴 열거와 `WM_COMMAND` dispatch로 고정했다.
- UAC prompt, 권한 부족 폴더, Shell 충돌 해결 UI, 장시간 Shell 진행률 UI, 실제 Explorer/Desktop 양방향 드래그앤드롭은 별도 수동 Windows 환경이 필요하다.
