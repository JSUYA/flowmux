<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# flowmux 구조 개선 실행 결과 (계획 대비 비교)

`docs/architecture-improvement-plan.md`의 7개 항목(#1–#7)을 모두 실행하고,
각 단계를 독립 커밋으로 만들었다. 본 문서는 개선 전/후를 수치로 비교하고,
실제 동작 검증 결과와 실행 중 새로 드러난 추가 개선 사항을 정리한다.

- 기준 커밋(개선 전): `2210f28`
- 브랜치: `refactor/arch-improvements`
- 검증 환경: DISPLAY=:1 (실제 X, Xorg), `dbus-run-session`, Rust stable

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
전 크레이트 통과. 유일한 실패는 **환경성 4건**(코드 회귀 아님):

- `ui::window::tests`의 agent-bar 가시성 4건. 원인: 테스트 빌더가
  `window.present()`를 호출하지 않아 실제 WM 하에서 `is_visible()==true` 단정이
  실패(CI의 xvfb는 WM 부재로 동기 가시화되어 통과). 본 리팩터가 건드리지 않은 로직.

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

실행 내내 현재 세션(PID 4066220)과 `/run/user/1000/flowmux.sock`는 무손상.
샌드박스 인스턴스만 해당 PID로 종료.

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
6. **GUI 테스트의 환경 민감성**: agent-bar 가시성 테스트 4건이 실제 WM에서 실패한다.
   테스트 빌더가 `present()` 후 매핑을 기다리거나 `is_visible` 대신 위젯 visible
   플래그를 직접 검사하도록 바꾸면 xvfb 밖에서도 결정적으로 통과한다.

## 결론

계획의 7개 항목을 모두 **동작 변경 0**(리팩터) / **동작 보강**(#5 degrade, #6 게이트)
원칙으로 실행했고, 단계별 커밋 + 자동화 회귀 + 리팩터 바이너리의 라이브 IPC 구동으로
검증했다. 코드 회귀는 없으며(유일 실패는 사전존재 환경성 4건), 위 6건의 후속
개선 여지를 남긴다.
