<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# flowmux 구조 개선 실행 결과 (계획 대비 비교)

`docs/architecture-improvement-plan.md`의 7개 항목(#1–#7)을 모두 실행하고,
각 단계를 독립 커밋으로 만들었다. 본 문서는 개선 전/후를 수치로 비교하고,
실제 동작 검증 결과와 실행 중 새로 드러난 추가 개선 사항을 정리한다.

> **사후 감사 정정:** 아래의 8개 커밋 표와 초기 수치는 해당 시점의 기록이다.
> 이후 전체 diff와 실제 DnD 행동을 다시 감사하면서 여러 회귀를 발견해 수정했다.
> 현재 결과와 최종 결론은 문서 끝의 **사후 감사 addendum**가 우선한다.

- 기준 커밋(개선 전): `2210f28`
- 브랜치: `refactor/arch-improvements`
- 초기 검증 환경: DISPLAY=:1 (실제 X, Xorg), `dbus-run-session`, Rust stable

## 커밋 단위 (8 커밋)

| 순서 | 커밋 | 항목 |
|---|---|---|
| 1 | `32a69a0` | #1-1 WindowController impl → 관심사별 서브모듈 분산 |
| 2 | `fa5325e` | #1-2 file-browser / agent-bar 필드 클러스터 추출 |
| 3 | `6f15bd5` | #4 CLI main.rs → 커맨드 모듈 + 테스트 외부화 |
| 4 | `46b01aa` | #3 GUI IPC 디스패치 verb-group 분할 |
| 5 | `769a547` | #2 Pane 트리 surface finder 통합 |
| 6 | `57e79d4` | #5 daemon panic → degrade 하드닝 |
| 7 | `56c7d43` | #6 flowmux-ssh 데드코드 feature 게이트 + 문서 정정 |
| 8 | `c20c163` | #7 core lib.rs 테스트 외부 분리 |

## 파일 규모 비교 (before → after)

| 파일 | before | after | 비고 |
|---|---:|---:|---|
| `ui/window.rs` → `ui/window/` | 10,638 (1파일) | mod.rs 8,836 + 5 서브모듈 (총 10,664) | 4,175줄 단일 impl 블록 소멸, 52개 메서드 5개 관심사 모듈로 분산 |
| `flowmux-cli/src/main.rs` | 3,260 | 882 | 핸들러 본문 5개 모듈 + 테스트(1,422줄) 외부화 |
| `flowmux/src/ipc_handler.rs` | 2,524 | 1,040 | 53-arm match → 5개 verb-group 메서드 + 라우터, 테스트(1,560줄) 외부화 |
| `flowmux-core/src/lib.rs` | 5,511 | 2,485 | 테스트(2,937줄) 외부화 + surface finder 통합(-150줄) |

## 항목별 실행 결과

### #1 WindowController 갓 오브젝트 분해 (P1) — 2단계 모두 수행
- **1단계 (커밋 1)**: `impl WindowController`의 52개 메서드를 순수 이동으로
  `window/{surface_ops,file_browser,agent_bar,polling,command_palette}.rs`에 분산.
  각 서브모듈은 `use super::*` + `impl WindowController`. 이동 메서드는 원래
  `window` 모듈 전역에서 접근되던 것이라 `pub(super)`로 가시성을 보존.
- **2단계 (커밋 2)**: 응집 필드 클러스터 추출. `FileBrowserState`(5필드→1),
  `AgentBarState`(2필드→1). `WindowController` 필드 **27 → 22**.
- **계획 주장 정정**: 계획서는 "필드 49개"라 했으나 실제 원본은 **27개**였다
  (`git show 2210f28` 확인). 수치를 실측값으로 정정한다.
- 검증: `cargo check -p flowmux` clean(기존 dead-code 경고 1건 외 0), GUI 테스트
  282 pass (변화 없음), 신규 clippy 경고 0.

### #4 CLI main.rs 커맨드 모듈 분할 (P2)
- 자유 함수 핸들러를 `keys.rs`(named-key 매핑), `request.rs`(Cmd→Request 빌더),
  `cmd_hooks.rs`, `cmd_ops.rs`, `output.rs`로 이동. 테스트 모듈은 `main_tests.rs`로
  `#[path]` 외부화. 이동 함수는 `pub(crate)`, 크레이트 루트 `use <module>::*`로
  모든 호출부(및 테스트 `use super::*`)를 무수정 유지.
- 검증: `cargo test -p flowmux-cli` 135 pass (변화 없음), clippy clean.

### #3 GUI IPC 디스패치 verb-group 분할 (P2)
- `handle`의 53-arm `match req`를 verb 그룹별 `impl GuiHandler` 메서드
  (`handle_{workspace,pane,browser,agent,notification}_verb`)로 위임.
  최상위 `handle`은 or-pattern 라우터(~55줄) + `_ => self.inner.handle(req)` 폴백.
  라우팅 exhaustiveness는 컴파일러가 검증하고, 라우팅/그룹 처리는 단일
  매핑에서 생성해 오라우팅 불가. 테스트는 `ipc_handler_tests.rs`로 외부화.
- 검증: ipc_handler 테스트 37 pass, GUI 코드 회귀 0.

### #2 Pane 트리 surface finder 통합 (P3)
- 4개 페이로드 세터(rename/browser_url/title_auto/cwd)가 각각 갖던 `*_descend`
  재귀 헬퍼를 제거하고 `Pane::find_surface_mut(target, surface) -> Option<&mut PaneSurface>`
  하나로 통합. `&mut Option<T>` take 우회 제거(페이로드를 값으로 소유해 직접 이동).
  구조 변이자(`add_surface_to_leaf_descend`, `split_leaf_descend`)는 유지.
- 동작 보존: `set_surface_title_auto`의 `$HOME` 조회는 surface를 찾은 뒤(Terminal+cwd)
  에만 실행 → leaf를 못 찾는 호출은 env 접근 비용 0 (기존 `home_cache` 지연과 동일).
- 검증: `cargo test -p flowmux-core` 118 pass (변화 없음), clippy clean.

### #5 임베디드 daemon panic 반경 경감 (P3)
- `state_store.rs`의 invariant `expect` 3곳(split pending / insert pending /
  destination existed)을 `let ... else { error!(...); return None; }`로 전환.
  이 메서드들의 기존 관례(degrade-to-None; 예: split_surface_into_pane의
  "guard rather than panic" 분기)와 일치. 트리가 어긋나도 GUI 프로세스 생존.
- **계획 대비 설계 판단**: 계획서는 "IPC 경계 `RpcError::Internal` 반환"을
  제안했으나, 대상 메서드는 전부 `Option` 반환이고 IPC 경계에서 None을 깨끗한
  응답으로 매핑한다. `Result`로 전환하면 호출 체인 전반을 바꾸는 침습적 변경이
  되어(계획의 "저위험" 원칙과 상충) 코드베이스 관례인 logged `return None`으로
  degrade했다. 관측성은 `tracing::error!`로 확보.
- 신규 테스트: 워크스페이스 간 `move_surface_to_pane`가 take→insert→dst_workspace
  경로(하드닝 지점)를 통과하는 케이스 추가(기존엔 same-ws/degrade 경로만 커버).
- 검증: `cargo test -p flowmux-daemon` 102 → **103** pass, clippy clean.

### #6 flowmux-ssh 데드코드 게이트 + 문서 정정 (P4)
- `SshClient`/`ClientHandler`(russh 네이티브 전송, `connect`는 Unimplemented,
  비-테스트 호출부 0 = 데드코드)를 `native` 모듈로 옮기고 off-by-default
  `native-ssh` feature로 게이트. `russh`/`russh-keys`/`async-trait`를 optional
  의존성으로 전환 → 기본 빌드에서 russh 제거. 전송 전용 `SshError` variant
  (Handshake/AuthFailed/Io/Unimplemented)도 동일 게이트, `ParseTarget`은 상시.
- 문서 정정: 크레이트 doc과 Cargo description이 포트포워딩/SFTP/네이티브
  핸드셰이크를 동작인 양 서술하던 것을 실제 상태(타깃 파서 + 시스템 ssh 주입)로
  정정. GUI/headless `SshConnect` 동작 불일치를 명시.
- 검증: `cargo check -p flowmux-ssh` (feature on/off 양쪽), `cargo test` 9 pass,
  clippy clean 양쪽, 워크스페이스/GUI 빌드 정상.

### #7 core lib.rs 테스트 외부 분리 (Optional)
- `#[cfg(test)] mod tests`(~2,937줄)를 `lib_tests.rs`로 `#[path]` 분리.
  lib.rs 프로덕션 코드 5,419 → 2,485.
- 검증: `cargo test -p flowmux-core` 118 pass (변화 없음).

## 전체 회귀 검증 (실제 동작 테스트)

### 자동화 테스트 — 전 워크스페이스 (`cargo test --workspace --no-fail-fast`)
초기 실행에서는 Agent Bar 4건을 환경성 실패로 분류했다. 사후 감사 결과 이 판단은
잘못됐으며, 사용자 `options.json`과 GTK parent/mapping 상태에 의존한 테스트 격리
문제였다. 최종 fixture 수정과 전체 결과는 addendum에 기록한다.

- 당시에는 테스트 빌더의 `window.present()` 누락과 WM 차이를 원인으로 판단했으나,
  이 분석은 폐기한다. 실제 원인은 사용자 옵션과 GTK mapping 상태에 의존한 fixture였다.

크레이트별: core 118 · cli 135 · daemon 103 · ipc 39 · notify 26 · procmon 10 ·
ssh 9 · state 16(+cross-process 1) · terminal 16 · vcs 5 · md-viewer 15 ·
GUI 282 (+4 환경성 실패).

### 라이브 앱 동작 테스트 (현재 세션 미종료)
리팩터된 GUI 바이너리를 **완전 샌드박스**(짧은 `XDG_RUNTIME_DIR` +
scratch state/data/config)로 별도 실행해 실제 IPC 왕복을 구동:

- `ping` → `"pong"`
- `workspace new` → `workspace_created` — GuiHandler.handle_workspace_verb(#3)
  → WindowController(#1) → StateStore(#5) → core 트리(#2) 전 경로 실동작,
  `tree`가 새 워크스페이스+pane 위젯 생성 확인.
- 잘못된 pane의 `read-screen` → 구조화된 `not_found`(패닉 없음, degrade 확인).

실행 내내 현재 세션(PID 4066220)과
`/run/user/1000/flowmux-4066220.sock`는 무손상.
보호 PID는 유지하고 샌드박스 인스턴스만 종료했다.

## 실행 중 드러난 추가 개선 사항

1. **#1 2단계 잔여 클러스터**: `badge_publisher_busy`/`badge_dirty`(폴링 배지 2필드),
   그리고 notifications/notifier 관련 필드도 서브구조로 묶을 여지가 있다(이번엔
   응집도가 가장 높은 FileBrowser/AgentBar만 추출). 남은 필드 22개 → 추가 축소 가능.
2. **`window/mod.rs`는 여전히 8,836줄**: 최대 잔존 덩어리는 `WindowController::dispatch`
   (~1,450줄 GtkCommand 매치)와 `WindowController::new`. dispatch도 #3과 동일한
   verb-group 위임으로 쪼갤 수 있다(후속 과제).
3. **CLI 리퀘스트 빌더 테스트 근접성**: `request.rs`로 옮긴 `build_request`류의
   테스트가 여전히 `main_tests.rs` 한 파일에 있다. 모듈별 인라인 `#[cfg(test)]`로
   더 가까이 둘 수 있다(탐색성 소폭 향상).
4. **CLAUDE.md 환경 문서 갭 (환경 이슈)**: 세션 중 시스템 `libthorvg-1.so.1` soname이
   C-API 없는 1.0.0 빌드를 가리켜(정상은 1.0.6) image-viewer 테스트 7건이 환경성
   실패했다. 코드와 무관(파일 바이트 동일). `scripts/install-thorvg.sh`가 soname을
   확정적으로 1.0.6에 고정하거나, 테스트가 C-API 부재를 skip 처리하도록 하면
   재발 방지가 된다. (검증 시엔 LD_LIBRARY_PATH shim으로 우회.)
5. **`daemon`의 헤드리스/GUI `SshConnect` 정렬**: #6에서 불일치를 문서화만 했다.
   headless를 GUI와 동일 의미(워크스페이스 + ssh 주입 상당)로 맞추는 것은 별도 과제.
6. **GUI 테스트 격리 (사후 감사에서 해결)**: Agent Bar 옵션을 fixture에서 고정하고
   `is_visible` 대신 위젯의 local `visible` 속성을 검사해 실제 WM에서도 결정적으로
   통과하도록 수정했다.

## 초기 결론 (사후 감사로 대체됨)

초기에는 “동작 변경 0 / 코드 회귀 없음”으로 결론 내렸으나, IPC smoke test만으로는
실제 pointer DnD와 저장·재시작 결합 경로를 검증하지 못했다. 아래 addendum가 이
결론을 철회하고 최종 감사 결과를 기록한다.

## 사후 감사 addendum (2026-07-10)

### 결론 정정

구조 분해의 방향과 CLI/API 호환성은 유지됐지만, 브랜치 전체를 regression-free로
판정한 초기 결론은 부정확했다. 표시 순서, pane/tab 상태 불변식, GTK DnD controller
중재에서 실제 회귀를 발견했으며 모두 수정 후 자동화·격리 행동 테스트를 다시 수행했다.
초기 파일 크기 표도 8커밋 snapshot이다. 사후 감사 완료 시점의 주요 파일은
`window/mod.rs` 9,068줄, CLI `main.rs` 881줄, `ipc_handler.rs` 1,038줄이다.

### 발견 및 수정

1. **워크스페이스 순서 end-to-end**
   - `workspace_order`의 누락·중복·stale ID와 stale active workspace를 정규화한다.
   - `workspace ls`, IPC `tree`, 저장·복원 Sidebar가 모두 표시 순서를 사용한다.
   - Sidebar 삭제의 `swap_remove`를 제거해 GTK 행과 내부 DnD 순서를 일치시켰고,
     선택된 행을 재삽입해도 highlight를 유지한다.
2. **pane/tab 상태 불변식**
   - same-pane 탭 이동이 singleton pane/workspace를 삭제하지 않게 in-place reorder한다.
   - non-`Tabs` 목적지를 변이 전에 거부해 source 탭 손실과 거짓 성공을 막는다.
   - singleton 탭을 자기 pane으로 split-drop하면 daemon/UI 모두 no-op으로 처리한다.
3. **#5 daemon 하드닝 재설계**
   - 초기 `pending.take()` + 사후 목적지 재검색 방식은 source 변이 뒤 실패할 수 있었다.
     최종 구현은 변이 전에 정확한 `Tabs` 목적지 인덱스를 확정하고 직접 insert/split해
     payload와 목적지가 갈라질 경로를 제거한다.
4. **워크스페이스 DnD MIME 충돌**
   - workspace와 tab drag가 모두 `String`을 광고해 같은 Sidebar 행의 tab target이
     workspace UUID를 먼저 받고 `missing separator`로 실패했다.
   - workspace drag에 `application/x-flowmux-workspace` MIME과 기존 `String`을 함께
     제공하고, Sidebar tab target만 workspace MIME을 거부한다. 기존 GTK String
     호환성은 유지한다.
5. **Sidebar tab MOVE의 중복 close와 dispatch 정지**
   - tab을 다른 workspace 행으로 옮긴 뒤 drag-end가 외부 창 이동으로 오인해 옛
     pane에 `CloseSurface`를 재전송했고, 잘못된 “Close workspace?” modal이 단일 GTK
     dispatcher를 막았다.
   - Sidebar와 DragSource가 동일한 seen/committed 상태를 공유하고 move ack 성공 뒤에만
     MOVE를 완료한다. `CloseSurface`도 surface membership을 confirmation보다 먼저
     검증해 stale command가 modal을 열 수 없게 했다.
6. **테스트·lint 기반 정정**
   - Agent Bar 4건은 옵션 fixture를 고정하고 local `visible` 속성을 검사하도록 바꿨다.
   - 추출된 테스트/모듈의 rustfmt 회귀와 Rust 1.95 all-target/all-feature clippy 오류를
     수정했다. CLI 기준 비교 79개 help, 83개 IPC request, 12개 hook 경로의 의미 차이는 0이다.

### 격리 라이브 검증

- 보호 대상은 PID `4066220`, 바이너리 `/home/jun/.cargo/bin/flowmux`, socket
  `/run/user/1000/flowmux-4066220.sock`이며 감사 전후 그대로 유지했다.
- 별도 target/prefix(`/tmp/flowmux-audit-20260710`)로 빌드·설치하고 Xephyr
  `DISPLAY=:91`, 격리 socket
  `/tmp/flowmux-audit-20260710/run2/runtime/flowmux.sock`에서만 조작했다.
- 실제 pointer drag로 gamma를 alpha 위에 옮겨 `[gamma, alpha, beta]`를 만들었다.
  gamma 선택 highlight, `workspace ls`, `tree`, `state.json` 순서가 일치했고 앱을
  종료·재시작한 뒤에도 같은 순서와 active gamma가 복원됐다.
- 반대 방향인 tab→workspace drop도 실제 수행했다. drag-end는 same-window target을
  인식했고 modal 없이 완료됐으며, 이동된 탭의 `close-tab`은 5초 제한 내 `ok`를
  반환하고 원래 한 탭씩의 tree로 복원됐다.
- 대표 IPC `ping`, `focus-pane`, `send-keys`/`read-screen`(`FLOWMUX_IPC_OK`),
  `split`/`close-pane`을 왕복 검증했다. in-app browser로 `https://example.com`을 열어
  ready-state, URL, title `Example Domain`, interactive snapshot까지 확인 후 닫았다.

### 최종 자동화 결과

- `cargo fmt --all -- --check`: 통과
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: 통과
- `cargo test --workspace --locked`: **930 passed, 17 ignored, 0 failed**
- `cargo build --release --workspace`: 통과
- 집중 회귀: workspace MIME arbitration, stale close after cross-workspace move,
  singleton self-split daemon/UI 테스트 모두 통과

GTK 전체 테스트는 성공했지만 일부 notification 테스트 경로가 Tokio runtime 없는 GTK
test executor에서 zbus background panic 로그를 남긴다. 프로덕션 앱은 Tokio runtime을
제공하고 이번 변경 경로와는 무관하나, 테스트 신호 품질을 위해 별도 정리가 필요하다.
또한 IPC bridge의 ack 대기에는 전역 timeout이 없어 dispatcher가 다른 이유로 막힐 경우
CLI가 오래 대기할 수 있는 잔여 위험이 있다.
