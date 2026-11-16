# network-monitor Architecture

## 개요

1. **범위**
    - 현재 소스 트리에 구현된 이 저장소의 구조: 워크스페이스 구성, 캡처 파이프라인, 디코더 체인, egress 모니터, 터미널 UI, 빌드 메모
    - `network-monitor`는 루트 패키지가 두 개의 터미널 frontend를 호스팅하는 단일 CLI 바이너리이고, 재사용 가능한 엔진 crate가 유일한 워크스페이스 멤버인 Cargo 워크스페이스다.
2. **문서**
    - 설치, 명령, 키: [README-ko.md](README-ko.md)
    - 영어 원문: [ARCHITECTURE.md](ARCHITECTURE.md)

---

## 1. 설계 원칙

1. **단계 분리**
    - 캡처, 디코드, UI 단계는 bounded crossbeam 채널로 연결된 별도 스레드에서 실행되므로 느린 터미널이 캡처를 멈추지 않고 패킷 폭주가 UI를 멈추지 않는다.
    - 각 단계는 무제한으로 쌓는 대신 bounded 채널로 backpressure를 적용한다.
2. **신뢰할 수 없는 wire 데이터**
    - 패킷은 panic 없이 디코딩한다. 모든 디코더는 인덱싱 전에 길이를 검증한다.
3. **오프라인, 로컬 우선**
    - 네트워크 클라이언트 의존성이 없으며 GeoIP와 ASN 보강은 로컬 `.mmdb` 파일만 읽는다.
    - 라이브 캡처는 인터페이스를 promiscuous 모드로 연다(`start_live_capture`).
4. **관찰 전용**
    - egress 모니터는 관찰하고 경보만 낸다. 트래픽을 차단하지 않는다.
5. **공유 store**
    - 하나의 `Store`(`rusqlite`를 통한 SQLite, bundled)가 mutex 뒤에 있으며 correlate/detect worker(쓰기)와 IPC 서버(읽기)가 공유한다.

---

## 2. 워크스페이스 구성

1. **Crate**
    - `network-monitor`(저장소 루트): 바이너리 패키지(`network-monitor`). CLI dispatch(clap), 캡처 파이프라인 연결, 두 ratatui frontend(캡처 TUI, inspector). 이 패키지의 `[workspace]` 표가 루트를 워크스페이스 루트로 만든다.
    - `network-monitor-core`(`crates/network-monitor-core`, 유일한 다른 워크스페이스 멤버): 라이브러리(`network_monitor_core`). 캡처, 디코더, flow 추적, 필터, 프로세스 귀속, 이름/GeoIP/identity 보강, SQLite store, 경보 엔진, 모니터 데몬, Unix 소켓 IPC
    - 루트 `Cargo.toml`이 공용 `[workspace.package]`와 `[workspace.dependencies]` 표를 유지하므로 core crate가 버전과 의존성 버전을 상속한다. 바이너리는 core crate에 경로와 crates.io용 버전을 함께 지정해 의존한다.
    - Edition 2024, `rust-version` 1.88(코드가 let chain을 쓴다), 버전 0.1.1, Apache-2.0이며 두 crate 모두 루트 `README.md`를 crates.io readme로 게시한다.
2. **플랫폼 gate**
    - core crate에서 `inspector`, `ipc`, `monitor`는 `#[cfg(unix)]`이며, `libc`는 두 crate 모두에서 Unix 전용 의존성이다.
    - 바이너리에서 `inspect`와 `monitor`, `inspect` 명령 본문은 `#[cfg(unix)]`이며, 다른 타깃에서는 그 명령이 오류를 반환한다.
    - `process`는 OS마다 구현이 하나씩(`linux.rs`, `macos.rs`, `windows.rs`) 있고 다른 타깃에서는 아무것도 반환하지 않는다.
    - Windows는 데몬 없이 캡처, 디코드, 분석, TUI 영역을 빌드한다.
3. **트리**

```markdown
network-monitor/
├─ Cargo.toml, Cargo.lock
├─ build.rs
├─ LICENSE.md
├─ README.md, README-ko.md
├─ ARCHITECTURE.md, ARCHITECTURE-ko.md
├─ .github/workflows/ci.yml, release.yml
├─ assets/icon/ (network-monitor.webp, network-monitor-small.webp, network-monitor.ico, network-monitor.rc, build_icon.py)
├─ docs/config.example.toml
├─ src/
│  ├─ main.rs
│  ├─ core/ (cli.rs, config.rs, permissions.rs)
│  ├─ inspect/ (mod.rs, state.rs, views.rs, glue.rs, tests.rs)
│  └─ tui/ (mod.rs, state.rs, theme.rs, i18n.rs, layout.rs, layout_store.rs, state_writer.rs, link_guard.rs, interaction.rs, format.rs, cards.rs, columns.rs, sort.rs, stats.rs, capture_io.rs, terminal.rs, views/)
└─ crates/network-monitor-core/
   ├─ Cargo.toml
   └─ src/
      ├─ lib.rs, app_dir.rs, debug.rs, error.rs, identity.rs
      ├─ alert/ (mod.rs, detectors.rs, baseline.rs)
      ├─ analysis/ (mod.rs, anomaly.rs, threat.rs)
      ├─ capture/ (mod.rs, live.rs, file.rs)
      ├─ decode/ (mod.rs, ethernet.rs, arp.rs, ipv4.rs, ipv6.rs, tcp.rs, udp.rs, icmp.rs, icmpv6.rs, dns.rs, tls.rs, ja.rs, http.rs, http2.rs, quic.rs)
      ├─ diff/, notify/, storage/ (mod.rs, ring.rs)
      ├─ enrich/ (mod.rs, geoip.rs, names.rs)
      ├─ filter/ (mod.rs, ast.rs, parser.rs, eval.rs)
      ├─ flow/ (mod.rs, tracker.rs, reassembly.rs, dns_tracker.rs, stats.rs)
      ├─ output/ (mod.rs, json.rs, log_writer.rs, pcap_writer.rs, pcapng_writer.rs)
      ├─ process/ (mod.rs, linux.rs, macos.rs, windows.rs)
      ├─ store/ (mod.rs, models.rs, schema.rs)
      └─ Unix 전용: inspector/, ipc/ (client, server, protocol), monitor/ (correlate, daemonize, paths)
```

```markdown
src/main.rs: 명령 dispatch, 디코드 스레드, JSON 출력 루프, 데몬과 inspector 진입점
src/core/cli.rs: 모든 명령과 옵션의 clap 정의
src/core/config.rs: 선택 사항인 config.toml(기본 인터페이스, 읽기만 하고 쓰지 않는 color_scheme, 저장 필터)
crates/network-monitor-core/src/app_dir.rs: 사용자별 폴더 하나인 ~/.config/network-monitor/(설정, 데몬 상태, 저장 캡처 로그)
src/core/permissions.rs: 캡처 권한 검사(Linux: root 필요, Windows: Npcap과 관리자 안내)
src/tui/: 두 TUI가 함께 쓰는 터미널 UI 키트와 캡처 화면(터미널 UI 참고)
src/inspect/: inspector TUI이며 일반 행 상태와 보기는 모든 플랫폼에서, 데몬 소켓 연결부는 Unix에서만 빌드된다
build.rs: Windows 리소스 삽입과, gnullvm 타깃에서 libunwind.dll 복사
assets/icon: WebP 원본 두 개, 생성된 ICO, 리소스 스크립트, ICO 재생성 스크립트
docs/config.example.toml: config.toml의 모든 키와 기본값
crates/network-monitor-core: 엔진 라이브러리이며 워크스페이스 멤버
```

---

## 3. 캡처 파이프라인

1. **라이브 경로**
    - bounded crossbeam 채널(각 용량 10 000)로 연결된 세 개의 스레드

```markdown
pcap interface
  |
  v
[capture thread]  capture::live::start_live_capture
  |  RawPacket  (bounded channel, cap 10_000)
  v
[decode thread]   decode_thread
  |   decode::decode_packet  -> flow::tracker::update  -> process attribution
  |  DecodedPacket  (bounded channel, cap 10_000)
  v
[UI thread]       tui::run_tui   (or json_output_loop when --json)
     PacketRing (bounded) + ratatui render loop (redraw on change, 1 s idle)
```

2. **단계**
    - capture 스레드는 libpcap 레코드를 `RawPacket`으로 변환하고 공유 `AtomicBool`에서 멈춘다.
    - decode 스레드는 계층형 디코더 체인을 실행하고, flow 상태(재조립, RTT, 재전송)를 갱신하며, 소유 프로세스를 붙인다.
    - UI 스레드는 디코딩된 패킷을 bounded `storage::ring::PacketRing`으로 비우고, 상태가 바뀌었거나 1초 유휴 간격이 지나면 다시 그리며 입력을 최대 50 ms 기다린다.
    - pcap writer(`-w`)와 로그 내보내기는 각자의 스레드에서 실행되고, UI 스레드는 패킷 복사본만 큐에 넣는다.
3. **JSON 모드**
    - `--json`은 TUI를 `json_output_loop`로 바꾸며, 이 루프는 JSON lines를 쓰고 선택적인 개수(`-c`) 또는 시간(`-G`) 제한에서 멈춘다.
    - `-w`와 이 제한은 서로 다른 모드에 속한다. `-w`는 TUI에만, `-c`와 `-G`는 JSON 루프에만 전달된다.
4. **오프라인 경로**
    - `read`는 라이브 capture 스레드를 `capture::file::start_file_capture`로 바꾼다. BPF 필터는 libpcap이 파일에 적용하고 나머지 파이프라인은 같다.
5. **방향**
    - TUI는 인터페이스의 로컬 IP 주소로 패킷을 IN 또는 OUT으로 분류하며, 주소가 없으면(파일 읽기) 사설/공인 주소 휴리스틱으로 대체한다.

---

## 4. 디코더 체인

1. **체인**
    - `decode::decode_packet`은 링크 계층부터 위로 `DecodeResult` / `NextDecode` 체인을 따라간다.
    - 각 프로토콜 디코더는 `Option`을 반환하고, 슬라이싱 전에 모든 길이를 검증하며, saturating 산술을 사용하므로 잘못되었거나 잘린 패킷은 panic 대신 체인을 깔끔하게 멈춘다.
2. **지원 계층**
    - Ethernet, ARP, IPv4/IPv6, TCP, UDP, ICMP/ICMPv6, DNS(질문과 응답), TLS(SNI와 JA3/JA4 fingerprint), HTTP/1.1, 평문 HTTP/2(HPACK), QUIC long header 감지
    - HPACK Huffman 리터럴은 풀지 않으며 QUIC Initial 복호화는 구현되지 않았다.
3. **요약**
    - `compute_summary`는 TUI가 쓰는 사람이 읽는 요약과 색상 힌트를 만든다.

---

## 5. 모듈 역할

1. **캡처와 디코드**
    - `capture`: libpcap 라이브 및 오프라인 캡처, 공유 `capture_loop`, `Linktype` 매핑
    - `decode`: 프로토콜별 디코더, `DecodedPacket`과 `Layer` 모델, 요약 계산
    - `flow`: `FlowTracker`(정규화된 5-tuple `FlowKey`를 키로 하는 LRU 캐시), TCP 재조립, DNS 질의/응답 짝짓기, RTT 추정
    - `filter`: nom 기반 디스플레이 필터 파서, `FilterExpr` AST, 패킷별 `eval_filter`
    - `storage`: TUI 스크롤백을 뒷받침하는 bounded 링 `PacketRing`
2. **분석과 귀속**
    - `analysis`: 보수적인 평문 이상 징후 검사(`anomaly`)와 상태를 가진 정찰 추적기(`threat`: 포트 스캔, 호스트 스윕, 드문 목적지)
    - `process`: 5-tuple을 PID, 이름, 실행 파일 경로로 매핑하는 OS 소켓 테이블 조회(Linux, macOS, Windows)
    - `enrich`: `Enricher` trait 뒤의 패시브 이름 해석(DNS 응답과 TLS SNI)과 오프라인 GeoIP/ASN 조회
    - `identity`: 경로, mtime, 크기를 키로 하는 LRU 캐시를 가진 바이너리 identity(macOS는 코드 서명, 그 외에는 실행 파일의 SHA-256)
3. **Egress 모니터 (Unix)**
    - `store`: 매개변수화된 쿼리를 쓰는 SQLite 데이터 접근 계층이며 schema와 model이 함께 있다.
    - `alert`: 다섯 signal 경보 엔진과 EWMA 송신량 baseline
    - `monitor`: 상시 데몬(캡처, correlate/detect worker, `run`/`status`/`stop`, daemonize, 런타임 경로)
    - `ipc`: `inspect`가 쓰는 Unix 소켓 요청/응답 프로토콜, 서버, 클라이언트
    - `inspector`: IPC 이벤트를 inspector 보기 상태로 바꾸는 reducer
    - `notify`: best-effort 데스크톱 알림(macOS `osascript`, Linux `notify-send`)과 stderr 로깅
4. **출력과 보조**
    - `output`: JSON lines, 평문 로그 writer, `PacketSink` trait 뒤의 pcap/pcapng writer
    - `diff`: 내용 기반 pcap 비교
    - `error`: 공유 `NetworkMonitorError` enum
    - `debug`: `NETWORK_MONITOR_DEBUG`로 켜는 opt-in 추적

---

## 6. Egress 모니터

1. **스레드**
    - `monitor run`은 capture 스레드와 `ipc-server` 스레드를 시작하고 호출 스레드에서 correlate/detect worker를 실행한다.
    - capture에서 worker로 가는 채널은 bounded(10 000)다.
2. **flow별 처리**
    - worker는 5-tuple을 PID, identity, 목적지 이름에 correlate하고, 오프라인 국가/ASN으로 보강해 `FlowEvent`를 만든다.
    - `AlertEngine::evaluate_full`은 이벤트 하나에 대해 모든 detector를 하나의 SQLite 트랜잭션 안에서 실행한다.
    - 프로세스 귀속이 없는 flow도 목적지로는 기록되지만 프로세스별 signal은 발생시키지 않는다.
3. **Baseline**
    - 첫 이벤트가 학습 구간을 시작한다(기본 7일, `--demo`는 5초).
    - 구간 동안에는 baseline만 채우고 경보를 내지 않으며, 그 이후 새로운 관찰이 경보를 발생시킨다.
4. **Signal**
    - 새 프로세스의 목적지: 프로세스에 대해 새로운 조직, 등록 가능 도메인, 또는 IP 라벨(notice)
    - 새 프로세스의 첫 외부 접속: 실행 파일의 첫 outbound 연결(warning)
    - 새 국가 / ASN: 프로세스에서 처음 본 국가 또는 ASN(notice)
    - 송신량 급증: 60초 구간당 outbound 바이트이며 alpha 0.3의 EWMA baseline을 쓴다. z가 4.0 이상, 5 MiB 이상, 샘플 8개 이상이면 발생한다(warning, 평균의 10배 이상이면 critical).
    - 프로그램 변조: baseline 이후 identity가 바뀐 뒤 외부로 접속(critical). 같은 team과 signing identity로 재서명한 업데이트는 `flag_signed_updates`가 켜졌을 때만 표시되며 데몬은 이를 꺼 둔다.
5. **Cooldown**
    - 같은 `dedup_key`의 경보는 이전 경보가 cooldown(기본 1시간)보다 새로우면 버려진다.
6. **IPC**
    - 데몬은 상태 디렉터리의 Unix 소켓에서 대기하며 정상 종료 시 소켓 파일을 지운다.
    - CLI가 쓰는 요청: `Status`, `LiveConnections`, `RecentAlerts`, `RecentConnections`, `Subscribe`, `Stop`. 구독한 클라이언트는 push 이벤트를 받는다.
    - `monitor status`, `monitor stop`, `inspect`, `inspect --json`은 이 프로토콜의 얇은 클라이언트다.
7. **Daemonize**
    - `--daemonize`는 double fork 후 `setsid`를 호출하고 stdout과 stderr를 상태 디렉터리의 `monitor.log`로 돌린다.
8. **신호와 종료**
    - `SIGINT`와 `SIGTERM`은 공유 중지 플래그를 설정하고, `monitor stop`은 IPC로 요청한다.

---

## 7. 터미널 UI

1. **모듈 구성**
    - `src/tui/`는 두 화면이 함께 쓴다. `theme.rs`(Indexed 색 팔레트), `i18n.rs`(en/ko 카탈로그), `layout.rs`(분할 비율과 카드 순서), `layout_store.rs`(상태 파일), `state_writer.rs`(상태 저장 스레드), `link_guard.rs`(심볼릭 링크 해석과 root 가드), `interaction.rs`(제스처, 주입 시계), `format.rs`(표시 폭 기반 텍스트), `cards.rs`와 `columns.rs`(카드 id, 적중 영역, 열 정의), `sort.rs`(타입 정렬 키), `views/`(`chrome.rs` 앱 바·탭·하단·도움말, `common.rs` 카드·표·스크롤바, 캡처 보기)로 이루어진다.
    - `state.rs`는 캡처 상태를 가지며 키나 마우스 전이마다 `Command`를 반환하고, `mod.rs`가 I/O 명령을 백그라운드 스레드로 보낸다.
    - 보기는 `&mut App` 또는 `&mut Inspect` 위의 순수 렌더 함수이며 적중 영역만 기록한다. 적중 영역은 매 프레임 시작에 초기화된다.
2. **터미널 수명 주기**
    - `terminal.rs`는 raw 모드, alternate screen, 마우스 캡처, 숨긴 커서를 함께 켜고, 하나의 멱등 복원을 공유하는 `Drop` 가드와 panic hook에서 복원한다.
    - 공용 이벤트 루프(`TuiHost`)는 상태가 바뀌었거나 1초 유휴 간격이 지나면 다시 그리고, 그리기 전에 쌓인 입력을 모두 처리하며, 키 해제 이벤트를 무시하고, 대기 시간을 다음 타이머(알림 만료나 길게 누름 기준 시간)까지로 줄인다.
3. **캡처 TUI**
    - 레이아웃: 앱 바, 필터 입력줄, 옆에 상세 카드가 붙은 패킷 목록, hex 카드, 처리량 줄(화면 높이가 24행 이상일 때만), 하단
    - 목록, 상세, hex는 두 그룹의 카드다. `CaptureRows`(위쪽 주 영역과 hex, 크기 조절 가능)와 `CaptureCols`(목록과 상세, 크기 조절과 순서 변경 가능)다.
    - 패킷 목록은 링 전체 위에서 보이는 행만 그리며, 정렬은 보이는 인덱스를 타입 키로 정렬한다(캐시하며 패킷이 들어오는 동안 최대 500 ms마다 다시 만든다).
    - 전체 영역 통계 보기(top talkers, 프로토콜 분포, flows, 타임라인, follow-stream, 북마크)는 최대 500 ms마다 다시 만드는 캐시를 읽는다.
    - 입력 모드: `Normal`, `FilterInput`, `Search`, `ColumnConfig`. `LIVE`는 보기를 최신 패킷에 고정하고 수동 이동은 `BROWSE`로 전환한다.
4. **Inspector TUI**
    - 다섯 탭(Connections, Processes, Domains, Alerts, History)이며 각 탭은 순서를 바꿀 수 있는 한 그룹 안의 표 카드와 상세 카드다.
    - core reducer를 일반 행(`inspect::state`)으로 변환하므로 상태와 보기는 모든 플랫폼에서 컴파일되고 테스트된다. `inspect::glue`(IPC 리더와 쿼리 스레드, host)만 `#[cfg(unix)]`이다.
    - 마지막 상태를 유지하고 데몬이 사라지면 연결 끊김 표시를 보여 준다.
5. **표시 언어**
    - 사용자에게 보이는 모든 문자열은 하나의 매크로 표에 영어와 한국어 문구를 함께 가진 `Msg` 항목이다. 기본은 영어이고 `L`은 재시작 없이 전환한다.
    - 데이터를 담는 값(개수, 이름, 경로, 엔진의 오류)은 형식 함수를 거치며 번역하지 않는다.
    - 정렬은 타입 키를 쓰므로 언어를 바꿔도 행 순서와 선택 위치가 바뀌지 않는다.
6. **표**
    - 표 카드는 자연 내용 폭으로 열 폭을 정한다. 고정 열은 폭을 유지하고, 글자 열은 가장 넓은 것부터 3칸까지 줄고, keep 표시가 없는 가장 오른쪽 열이 마지막에 숨으며, 사용자가 끈 폭은 가장 나중에 줄고 512칸으로 제한된다.
    - 머리글 클릭과 `s`/`S`는 하나의 정렬 순환(숫자·시간은 내림차순 먼저, 문자는 오름차순 먼저, 세 번째는 캡처 순서)을 공유하며, 다시 정렬해도 선택은 같은 항목을 따라간다.
    - 스크롤바는 오른쪽 테두리 안쪽 거터 열에 회색 계열로 그리며 포커스 색을 쓰지 않고, ` a-b / total `은 아래 테두리에 그린다.
7. **배치**
    - 카드 제목을 길게 누르면(400 ms, 주입 시계를 쓰는 `Ui::tick`이 측정) 이동 모드로 들어간다. 순서는 실시간으로 미리 보이고, 놓으면 확정되며, `Esc`, 탭 전환, 크기 변경은 이전 순서로 되돌리고, 버튼 없는 마우스 이동이나 새 누름은 놓침을 확정으로 처리한다.
    - 경계는 비율로 저장하며 이웃 카드의 최소 크기로 제한한다. 칸 크기는 칸에 남는다.

---

## 8. 상태와 파일

1. **메모리 상태**
    - 캡처 스크롤백: `tui::App`의 `PacketRing`(bounded)이며 UI 세션 동안 유지된다.
    - flow 테이블: `flow::FlowTracker`의 `FlowKey`에서 `FlowState`로 가는 LRU 캐시이며 캡처 세션 동안 유지된다.
    - 라이브 connections: 모니터 worker의 `Mutex<LiveState>`이며 데몬 수명 동안 유지된다.
2. **영속 상태**
    - 사용자별 폴더 하나: `~/.config/network-monitor/`(홈에 `.config`를 붙인 경로이며 Windows에서는 `%USERPROFILE%\.config`이다). 경로는 한 곳인 `network_monitor_core::app_dir`에서 정한다. `home_dir`은 `USERPROFILE`, `HOME` 중 비어 있지 않은 첫 값을 쓰고, `app_dir_in`은 홈 경로를 인자로 받으므로 테스트가 실제 프로필을 건드리지 않는다. 대체 폴더는 없다. 홈 디렉터리가 없으면 TUI는 stderr에 한 줄을 출력하고 상태와 로그 저장을 끄며, `monitor::paths::resolve`는 `--state-dir`이 없으면 실패한다.
    - 설정 파일: 그 폴더의 `config.toml`(읽기 전용이며 프로그램이 쓰지 않는다). 이 파일이 없고 이전 `<config_dir>/network-monitor/config.toml`이 있으면 시작 시 새 경로를 알리는 안내를 한 줄 출력한다.
    - 데몬 상태: 그 폴더 또는 `--state-dir`로 지정한 디렉터리의 `network-monitor.sock`, `network-monitor.db`, `network-monitor.pid`, `monitor.log`. 이전 `<data_local_dir>/network-monitor`가 있고 새 폴더에 데이터베이스가 아직 없으면 `monitor::paths::resolve`가 두 경로를 담은 안내를 한 줄 출력하며, 데이터는 옮기지 않는다.
    - 저장 캡처 로그: 그 폴더의 `network-monitor-<YYYYMMDD-HHMMSS>.log`이며 `tui::capture_io::write_log_in`이 백그라운드 스레드에서 쓴다(`w` 키). root로 실행 중이면 경로 어디서든 다른 사용자가 소유한 심볼릭 링크를 거부하고, 홈 디렉터리가 없으면 오류 알림과 함께 내보내기가 실패한다.
    - TUI 상태: 그 폴더의 `tui.toml`이며 두 TUI가 공유한다(`tui::layout_store`). `version`, `language`, 카드 순서, 분할 비율, 열 폭을 안정적인 그룹·카드 이름 키로 저장한다. 저장은 상태 저장 스레드(`tui::state_writer`)가 맡는다. UI 스레드가 확정 이벤트에서 파일 내용 전체를 만들어 큐에 넣고, 한 경로에 몰린 요청은 최신 내용 하나로 합치며, 쓰기 실패는 타입 있는 메시지와 오류 알림으로 돌아오고, 종료 경로는 스레드를 최대 5초 기다린 뒤 마지막 쓰기가 실패했거나 아직 진행 중이면 stderr에 경고 한 줄을 출력한다. 쓰기는 원자적이며(대상 폴더에 배타적으로 만든 임시 파일에 쓰고 sync한 뒤 이름 바꾸기) 확정 이벤트에서만 일어나고, 심볼릭 링크 연쇄(최대 16단계, `tui::link_guard`)를 먼저 따라가 링크가 유지되며, 유효 사용자 ID 0인 프로세스는 다른 사용자가 소유한 링크를 거부한다. 읽을 수 없거나 다른 버전인 파일은 첫 덮어쓰기 전에 `tui.toml.bak`으로 복사하고, 시작할 때 만든 지 60초 이상 된 `tui.toml.<pid>.tmp`만 지우며(더 어린 파일은 다른 인스턴스가 저장 중일 수 있다), 잘못된 항목은 건너뛰고, 열 폭은 512칸으로 제한한다. 테스트는 경로, 시계, 소유자 조회를 주입한다.

---

## 9. 디스플레이 필터

1. **문법**
    - 프로토콜 atom, 필드 비교(`==`, `!=`), `contains`, `threat`/`alert`, 불리언 조합(`and`/`&&`, `or`/`||`, `not`/`!`, 괄호), 자유 텍스트 대체이며 필드 목록은 [README-ko.md](README-ko.md)에 있다.
    - `and`가 `or`보다 먼저 묶인다.
2. **파서**
    - 파서는 parse 시점에 정규 점 구분 필드 경로를 한 번 만들므로 패킷별 평가가 경로를 위해 할당하지 않는다.
    - 완전한 구조화 식이 아닌 입력은 패킷 요약에 대한 대소문자 무시 일치인 `FreeText`가 되며, 빈 필터만 오류다.
3. **평가**
    - `tcp.port`와 `udp.port`는 양쪽 중 어느 쪽이든 일치하며, 이들에 대한 `!=`는 양쪽 모두 달라야 한다.

---

## 10. 빌드 메모

1. **네이티브 의존성**
    - `pcap` crate는 libpcap을 링크한다. Unix는 `pkg-config`, Windows는 `wpcap`이며 `LIB`(또는 `LIBPCAP_LIBDIR`)를 Npcap SDK의 `Lib\x64` 폴더로 지정한다.
    - SQLite를 소스에서 컴파일하므로(`rusqlite`의 `bundled` feature) C 컴파일러가 필요하다.
2. **`build.rs` (루트 패키지)**
    - Windows 타깃에서는 `OUT_DIR`에 manifest(`asInvoker`)와 리소스 스크립트(아이콘은 `CARGO_MANIFEST_DIR/assets/icon/network-monitor.ico`, 영어 `VERSIONINFO` 블록 `040904B0`과 translation `0x409`, 값은 `CARGO_PKG_*`)를 생성하고 `windres`로 컴파일한다. 후보는 링커 폴더, `PATH` 순이며 타깃 아키텍처별로 `<arch>-w64-mingw32-windres`, `llvm-windres --target=<arch>-w64-mingw32`, `windres`이다.
    - 각 오브젝트의 첫 두 바이트(COFF 머신 타입)가 타깃 아키텍처와 같아야 하므로 `aarch64-pc-windows-msvc` 빌드가 x64 오브젝트를 링크하는 일은 없으며, 맞지 않는 후보는 건너뛴다.
    - 쓸 수 있는 `windres`가 없으면 경고를 출력하고 리소스 없이 빌드한다.
    - `gnullvm` 타깃에서는 `libunwind.dll`을 빌드된 실행 파일 옆에 복사한다.
    - 패키징된 crate에는 `build.rs`, `assets/icon/network-monitor.ico`가 들어가고 `.webp` 원본과 `build_icon.py`는 제외되므로 빌드에 이미지 도구가 필요 없다.
3. **아이콘**
    - `assets/icon/network-monitor.webp`(1024 px 타일)는 ICO의 40~256 px 항목을, `network-monitor-small.webp`(256 px 육각형 크롭)는 16~32 px 항목을 만든다.
    - `build_icon.py`가 ImageMagick으로 `network-monitor.ico`를 다시 만든다. 원본이 바뀔 때만 실행한다.

---

## 11. 검증 전략

1. **명령**
    - 변경된 영역을 덮는 가장 좁은 명령을 사용한다.

```bash
cargo fmt --check
cargo clippy --workspace --all-targets
cargo test --workspace
cargo build --workspace
```

2. **범위**
    - core crate는 디코더, flow tracker, 필터, 링, writer, 경보 엔진, IPC의 단위 테스트를 가지며, 바이너리 crate는 모든 보기를 영어와 한국어로 60x16, 100x30, 130x40에서 그리는 `TestBackend` 버퍼 테스트(포커스, 스크롤바, 선택 띠, 알림, 칩의 정확한 셀과 실제 마우스 이벤트를 통한 적중 영역), 길게 누름·드래그·놓침에 대한 시계 주입 테스트, 정렬 순환·열 폭 결정·분할 비율·상태 저장소·상태 저장 스레드(합치기, 제한 시간 합류)·링크 가드(연쇄, 루프, root 거부)의 단위 테스트, 글리프 예산과 이중선이 오버레이에만 나타나는지에 대한 버퍼 테스트, 그리고 설정 테스트(주입한 홈 디렉터리, `docs/config.example.toml` 파싱)를 가진다.
    - `monitor`, `ipc`, `inspector` 모듈은 Unix 타깃에서만 컴파일되고 실행되며, `notify`는 어디서나 컴파일되지만 OS 알림은 macOS와 Linux에서만 보낸다.
