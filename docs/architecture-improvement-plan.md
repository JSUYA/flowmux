<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# flowmux 구조 개선 계획서

## 개요

현재 코드베이스 구조에서 개선 여지가 있는 항목 7개를 도출하고, **Claude Code**와 **Codex(GPT-5 계열)** 두 리뷰어가 각각 실제 소스(파일:라인)에 근거해 교차 검증했다. 본 문서는 그 교차 검증 결과와, 유효하다고 판단된 항목의 실행 계획을 담는다.

- 대상: `/home/jun/dev/os/flowmux` (Rust 워크스페이스, GTK4 GUI)
- 검증 방식: 두 리뷰어 독립 판정 → 불일치 항목은 실코드 재확인으로 확정
- 원칙: 과장된 주장은 축소하고, 실제 의미(유지보수·정확성·정직성) 있는 것만 남긴다

### 검증 중 뒤집힌 주장 (중요)

초기 도출본 대비 교차 검증에서 **2건이 실질적으로 조정**되었다. 이 문서의 판정이 최종본이다.

- **#2 (트리 재귀 통합)**: "헬퍼 24개 → 1개"는 과장. 실제로는 읽기 전용 finder와 leaf 페이로드 변이자 6~8개만 하나의 제네릭 finder로 접힌다. split/remove/형제탐색 같은 구조 변이자는 제어 흐름·소유권 제약이 달라 별개로 유지된다.

---

## 교차 검증 판정 요약

| # | 항목 | Claude | Codex | 최종 판정 | 의미 | 우선순위 |
|---|---|---|---|---|---|---|
| 1 | `WindowController` 갓 오브젝트 | VALID | VALID | **VALID** | 유의미 (높음) | **P1** |
| 2 | Pane 트리 재귀 헬퍼 분산 | 부분(과장 축소) | PARTIAL | **PARTIALLY-VALID** | 유의미 | P3 |
| 3 | IPC verb 디스패치 이중화 | 부분(과장 축소) | PARTIAL | **PARTIALLY-VALID** | 유의미(주로 탐색성) | P2 |
| 4 | `flowmux-cli/main.rs` 단일 3.3k줄 | VALID | VALID | **VALID** | 유의미 | P2 |
| 5 | 임베디드 daemon panic 반경 | VALID(저확률) | PARTIAL(저확률) | **PARTIALLY-VALID** | 유의미, 저확률 | P3 |
| 7 | `flowmux-core/lib.rs` 인라인 테스트 | VALID(사소) | VALID(cosmetic) | **VALID (cosmetic)** | 낮음 | Optional |

## 항목별 실행 계획

### P1 — #1. `WindowController` 갓 오브젝트 분해

**근거**
- `crates/flowmux/src/ui/window.rs` ≈ 10,638줄
- `struct WindowController` 필드 **49개** (`window.rs:198`)
- 단일 `impl WindowController` 블록 ≈ **4,200줄**, 메서드 **85개** (`window.rs:418`–`4629`)
- 한 타입이 file browser 상태, agent bar, notifications, cwd 폴링, command palette, workspace 렌더, resize handle, tear-off, focus MRU를 모두 소유

**판정**: VALID, 최우선. 두 리뷰어 일치.

**Codex 보강**: 파일 분할만으로는 49-필드 중앙 객체가 그대로 남는다. 응집된 상태 클러스터도 함께 서브구조로 추출해야 실질 개선.

**작업 범위**
1. (1단계, 저위험) `impl WindowController`를 관심사별 여러 impl 블록으로 쪼개 파일 분산. Rust는 동일 타입 impl의 파일 분산을 허용.
   - `window/file_browser.rs` — `file_browser_*`, `show_file_browser_for_pane`, `focus_file_browser` 등
   - `window/agent_bar.rs` — `refresh_agent_bar`, `mark_agent_bar_attention`, `open_agent_bar_item` 등
   - `window/polling.rs` — `poll_agent_processes`, `poll_terminal_cwds`, `install_cwd_polling_fallback` 등
   - `window/command_palette.rs` — `show_command_palette`, `run_command_palette_command`, `run_project_command` 등
   - `window/surface_ops.rs` — split/move/tear-off/reattach 계열
2. (2단계, 후속) 관련 필드를 서브구조로 묶어 필드 49개 축소.
   - `file_browser_source_pane` / `file_browser_active` / `file_browser_pane_states` / `file_browser_split` / `file_browser` → `FileBrowserState`
   - `agent_bar` / `agent_bar_attentions` → `AgentBarState`
   - 폴링/badge 관련(`badge_publisher_busy`, `badge_dirty`) → 폴링 서브구조

**검증**: `cargo check -p flowmux` 통과, `cargo test -p flowmux` 그대로 통과(동작 변경 없음). 1단계는 순수 이동이므로 diff는 커도 의미 변경 0.

**리스크**: 낮음(1단계). 2단계는 필드 접근 경로가 바뀌므로 컴파일러가 잡아준다.

---

### P2 — #4. `flowmux-cli/src/main.rs` 커맨드별 모듈 분할

**근거**
- `crates/flowmux-cli/src/main.rs` ≈ 3,260줄, 함수 84개
- 서브커맨드 디스패치와 요청 빌드가 인라인: `match &cmd`(`main.rs:756`), `build_request`(`main.rs:1051`, `1057`)

**판정**: VALID, 기계적. 두 리뷰어 일치.

**작업 범위**: clap `Cmd` enum은 이미 구조가 있으므로 핸들러 본문만 `cmd/` 하위 모듈로 이동(`cmd/workspace.rs`, `cmd/pane`, `cmd/browser.rs` 등). `main.rs`는 파싱 + 디스패치만 남긴다.

**검증**: `cargo check -p flowmux-cli`, `cargo test -p flowmux-cli`(테스트 129개) 통과.

**리스크**: 낮음.

---

### P2 — #3. GUI IPC 디스패치 match 그룹 분할

> 걷어낸 것: "verb 3곳 수정 드리프트" + 공유 `trait VerbHandler`. daemon이 GTK verb를 `Unimplemented`로 반환하는 것은 문서화된 의도된 설계(`handler.rs:3`)이므로 버그가 아니다. 트레이트는 과설계.

**근거**: GUI 디스패치가 `crates/flowmux/src/ipc_handler.rs`의 **118-arm 단일 match**(파일 ≈ 2,524줄, `ipc_handler.rs:116`~). 탐색·리뷰가 어렵다.

**작업 범위**: verb 그룹별로 GUI match를 서브함수로 위임(`handle_workspace_verb`, `handle_pane_verb`, `handle_browser_verb` …). 각 함수는 `Request`의 해당 그룹만 처리하고, 최상위 match는 그룹 위임만 남긴다.

**검증**: `cargo test -p flowmux`(IPC 관련 테스트) 통과. 동작 변경 0.

**리스크**: 낮음. 순수 리팩터.

---

### P3 — #2. Pane 트리 surface finder 통합

> 걷어낸 것: "헬퍼 24개 → 제네릭 walker 1개" 전면 통합. 구조 변이자(`split_leaf`, `add_surface_to_leaf`, `remove_leaf`, 오른쪽 형제/조상 탐색)는 소유권·제어 흐름이 달라 통합 대상 아님 — **그대로 둔다**.

**근거**: `_descend` 계열(`lib.rs:1308`~)은 골격 동일 — `Leaf(target) → Tabs → surface 탐색 → 변이 / else first||second 재귀`. 변이 본문만 다르고, `&mut Option<T>` take 패턴은 "match에서 1회 소비"를 우회하는 군더더기.

**작업 범위 (surface 페이로드 변이자 6~8개만)**: rename/set_url/set_cwd/set_title 등을 하나의 finder로 대체.
```rust
fn find_surface_mut(node: &mut Pane, pane: PaneId, surface: SurfaceId) -> Option<&mut PaneSurface>
```
호출부에서 반환된 surface를 직접 변이. `_descend` 함수들과 `&mut Option<T>` 우회 제거.

**가치**: 트리 재귀 버그(형제 순서, active 전파)는 미묘하므로 finder를 한 곳으로 모으면 표면 축소.

**검증**: `cargo test -p flowmux-core`(테스트 118개) 통과. 전후 동일.

**리스크**: 중. 테스트로 가드.

---

### P3 — #5. 임베디드 daemon panic 반경 경감

**근거**
- 임베디드 daemon은 GUI 프로세스에서 실행 → 상태 변이 panic = 앱 전체 종료
- 실 변이 경로의 invariant `expect`:
  - `state_store.rs:405` `expect("surface pending split")`
  - `state_store.rs:572` `expect("surface pending insert")`
  - `state_store.rs:580` `expect("destination existed before take")`

**판정**: PARTIALLY-VALID, 유의미하나 **저확률**. 두 리뷰어 일치. 세 지점 모두 선행 존재 검사(`find_leaf_content(...).is_some()`)와 store 락으로 가드되어 정상 흐름에선 도달 안 함.

**작업 범위**: 방어적 강화. invariant 위반 시 `panic` 대신 IPC 경계에서 `RpcError::Internal` 반환. 트리 상태가 어긋나도 창이 죽지 않고 degrade.

**검증**: 기존 테스트 유지 + 인위적 불변식 위반 시 에러 반환하는 단위 테스트 1개 추가.

**리스크**: 낮음. 값싼 하드닝.

---

### Optional — #7. `flowmux-core/lib.rs` 테스트 외부 분리

**근거**: `crates/flowmux-core/src/lib.rs` ≈ 5,511줄 중 `#[cfg(test)] mod tests`가 `lib.rs:2575`부터 끝까지 ≈ 2,936줄.

**판정**: VALID이나 **cosmetic**. 두 리뷰어 일치.

**작업**: `#[path = "lib_tests.rs"] mod tests;`로 분리. 프로덕션 파일 절반, 탐색 개선. 아키텍처 영향 없음.

**리스크**: 없음. 원할 때만.

---

## 실행 로드맵

```
P1  #1 WindowController 분해        → 나머지 작업의 탐색성 대폭 개선, 먼저 착수
      1단계 impl 파일 분산(저위험) → 2단계 필드 클러스터 추출
P2  #4 CLI 커맨드 모듈화 (기계적, 병렬 가능)
    #3 GUI IPC match 그룹 분할
P3  #2 트리 surface finder 통합 (테스트 가드)
    #5 daemon panic → RpcError 하드닝 (값쌈)
Opt #7 core 테스트 파일 분리 (여유 시)
```

**권장 착수점**: **#1 (P1)**. window.rs를 쪼개면 이후 모든 작업의 탐색·리뷰 비용이 내려간다. 1단계(순수 impl 이동)는 위험이 낮고 즉시 효과.

## 범위 밖 / 유지

- 테스트 커버리지는 양호(core 118, daemon 102, GUI 191, CLI 129). 이번 개선은 **구조** 문제이지 검증 부족이 아니다.
- daemon의 verb 부분 구현(50개 GUI-only가 `Unimplemented`)은 **설계상 의도**이므로 "버그"로 다루지 않는다.
- 사전 존재하던 데드 코드/스타일은 본 계획의 명시 항목(#6) 외에는 건드리지 않는다.

## 검증 방법 공통

각 항목 완료 기준:

```bash
cargo check                                   # 헤드리스 크레이트
cargo check -p flowmux                        # GUI 크레이트(GTK dev 패키지 필요)
cargo clippy --workspace --all-targets -- -D warnings
xvfb-run -a dbus-run-session -- cargo test --workspace --locked
```

리팩터 항목(#1·#3·#4·#7)은 **동작 변경 0**이 목표이므로, 기존 테스트가 수정 없이 통과해야 한다.
