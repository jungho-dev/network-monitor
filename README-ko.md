# network-monitor

## 개요

1. **언어**
    - 영어 원문: [README.md](README.md)
    - 문서의 기본 언어는 영어다.
2. **소개**
    - 터미널 패킷 분석기와 로컬 egress 모니터를 하나의 바이너리 `network-monitor`로 제공한다.
    - 패킷 분석기는 라이브 인터페이스에서 캡처하거나 PCAP/PCAPNG 파일을 읽어 트래픽을 디코딩하고 키보드 중심의 터미널 UI(TUI)로 보여 준다.
    - egress 모니터는 "내 머신이 무엇과 통신하고 있으며, 그중 새로운 것이 있는가?"에 답하는 백그라운드 데몬이다. 각 flow를 프로세스에 연결하고, 목적지 이름을 패시브하게 해석하고, 오프라인 국가/ASN 데이터로 보강하고, baseline을 학습한 뒤 경보를 발생시킨다.
    - 네트워크 클라이언트 의존성이 없다. GeoIP/ASN 데이터는 로컬 파일에서 읽으며 어떤 서비스로도 전송하지 않는다.
3. **특징**
    - BPF 필터를 쓰는 라이브 캡처, PCAP/PCAPNG 읽기와 쓰기, 스크립트용 JSON lines 출력, 두 캡처 파일의 내용 비교
    - Ethernet, ARP, IPv4/IPv6, TCP, UDP, ICMP/ICMPv6, DNS, TLS(SNI, JA3/JA4), HTTP/1.1, 평문 HTTP/2, QUIC long header 디코더
    - follow-stream 보기를 갖춘 TCP 스트림 재조립, RTT 추정, flow 표, top talkers, 프로토콜 분포, 연결 타임라인
    - `and` / `or` / `not`을 지원하는 디스플레이 필터, 이름 붙은 저장 필터, 정규식 검색, 북마크, 인바운드/아웃바운드 보기
    - Linux, macOS, Windows에서 flow별 프로세스 귀속
    - 다섯 가지 경보 signal을 갖춘 egress 모니터: 새 프로세스의 목적지, 새 프로세스의 첫 외부 접속, 새 국가 또는 ASN, 송신량 급증, 프로그램 변조
4. **플랫폼**
    - 패킷 분석기: Linux, macOS, Windows
    - egress 모니터(`monitor`, `inspect`): Unix 전용(Linux, macOS)이며 Windows에서는 이 명령이 오류로 종료된다.
    - 프로젝트는 macOS 우선으로 개발되며 Windows의 프로세스 귀속은 가볍게만 테스트되었다.
5. **문서**
    - 소스 구조와 내부 흐름: [ARCHITECTURE-ko.md](ARCHITECTURE-ko.md) (영어: [ARCHITECTURE.md](ARCHITECTURE.md))
    - 설정 예시: [docs/config.example.toml](docs/config.example.toml)
    - 라이선스: [LICENSE.md](LICENSE.md) (Apache-2.0)
6. **배지**
    - [![Crates.io](https://img.shields.io/crates/v/network-monitor.svg)](https://crates.io/crates/network-monitor) [![License: Apache-2.0](https://img.shields.io/badge/License-Apache--2.0-blue.svg)](LICENSE.md)

---

## 빠른 시작

### 1. 요구 사항

1. **Rust**
    - 빌드에는 Rust 1.88 이상이 필요하다(`Cargo.toml`의 `rust-version`, edition 2024).
    - SQLite를 번들로 포함하므로(`rusqlite`의 `bundled` feature) 모든 OS에서 C 컴파일러가 필요하다.
2. **Windows**
    - 실행: x64용 [Npcap](https://npcap.com/) 런타임, 캡처는 관리자 프롬프트에서 실행한다.
    - 빌드: Npcap SDK도 필요하다. `pcap` crate가 `wpcap`을 링크하므로 환경 변수 `LIB`(또는 `LIBPCAP_LIBDIR`)를 SDK의 `Lib\x64` 폴더로 지정한다.
    - 선택: 리소스 컴파일러(`<arch>-w64-mingw32-windres`, `llvm-windres`, 또는 `windres`). `build.rs`가 리소스 스크립트를 생성하고 이 도구로 앱 아이콘, 영어 버전 정보, `asInvoker` manifest를 exe에 넣는다. 다른 CPU 아키텍처용 오브젝트를 만드는 도구는 건너뛰며, 도구가 하나도 없으면 경고만 출력하고 빌드는 성공한다.
3. **Linux**
    - 빌드에는 libpcap 개발 패키지가 필요하다. Debian/Ubuntu는 `libpcap-dev`, Fedora는 `libpcap-devel`이다.
    - 캡처에는 root 또는 `CAP_NET_RAW` capability가 필요하다. `sudo`로 실행하거나 `setcap`으로 한 번 부여한다(문제 해결 참고).
4. **macOS**
    - 시스템 libpcap을 사용하며, 빌드하려면 명령줄 도구(`xcode-select --install`)를 설치한다.
    - macOS에서 앱은 권한을 직접 검사하지 않는다. 캡처 장치를 열 수 있는지는 OS가 결정하며 `sudo`가 가장 간단한 방법이다.
5. **권한이 필요 없는 명령**
    - `read`와 `diff`는 상승된 권한 없이 파일에서 동작하고, `list-interfaces`는 장치 목록만 출력한다.

### 2. 설치

1. **crates.io에서**
    - Windows에서는 먼저 `LIB`를 Npcap SDK의 `Lib\x64` 폴더로 지정한다.

```bash
cargo install network-monitor
```

```powershell
$env:LIB = "<Npcap SDK folder>\Lib\x64;$env:LIB"
cargo install network-monitor
```

2. **소스에서**
    - 저장소를 clone해 plain cargo로 빌드한다. Windows에서는 먼저 `LIB`를 같은 방식으로 지정한다.
    - 바이너리는 `target/release/network-monitor`(Windows는 `network-monitor.exe`)다.

```bash
git clone https://github.com/jungho-dev/network-monitor.git
cd network-monitor
cargo build --release
```

3. **워크스페이스**
    - 저장소 루트가 `network-monitor` 패키지이며 두 TUI를 포함한 CLI 바이너리다.
    - `crates/network-monitor-core`는 워크스페이스 멤버이며 재사용 가능한 엔진 라이브러리 `network-monitor-core`다.
    - 두 crate 모두 이 README를 crates.io readme로 게시한다.

### 3. 설정

1. **선택 사항인 설정 파일**
    - 없어도 실행된다. 설정 파일로 기본 인터페이스와 저장 필터를 지정한다.
    - 경로: 모든 OS에서 `~/.config/network-monitor/config.toml`이며, Windows에서는 `%USERPROFILE%\.config\network-monitor\config.toml`이다.
    - 홈 디렉터리는 `USERPROFILE`, `HOME` 중 비어 있지 않은 첫 값이며, 이 경로는 `$XDG_CONFIG_HOME`을 따르지 않는다.
    - 같은 폴더에 사용자별 나머지 파일(데몬 상태와 저장 캡처 로그)도 모두 들어간다. 설정 절을 참고한다.
2. **최소 예시**
    - 폴더와 파일은 직접 만든다. 프로그램이 대신 만들어 주지 않는다.
    - [docs/config.example.toml](docs/config.example.toml)에 모든 키와 기본값이 있다. 위 경로로 복사해 수정한다.

```toml
[capture]
default_interface = "en0"        # used when -i is omitted

[filters]                        # recall in the filter bar as :name
https = "tcp.port == 443"
dns   = "udp.port == 53"
```

```markdown
default_interface: -i를 생략했을 때 캡처에 쓰는 인터페이스
filters: 각 키는 저장된 디스플레이 필터이며 필터 입력줄에 :https를 입력하면 적용된다
모든 키는 설정 섹션에 정리되어 있다
```

### 4. 첫 실행

1. **인터페이스 확인**
    - 관찰할 인터페이스 이름을 고른다.

```bash
network-monitor list-interfaces
```

2. **라이브 TUI 시작**
    - `-i`가 없으면 `capture.default_interface`를 쓰고, 그것도 없으면 활성 인터페이스(up, running, connected, loopback 아님, 실제 IPv4 주소 보유)를 자동 선택해 stderr에 알린다.

```bash
sudo network-monitor
sudo network-monitor capture -i en0
```

```markdown
Windows에서는 sudo 없이 관리자 프롬프트에서 같은 명령을 실행한다
-i는 인터페이스를 지정하고 -f는 "tcp port 443" 같은 BPF 캡처 필터를 추가한다
```

3. **화면 읽기**
    - 위에서 아래로: 앱 바(인터페이스 또는 파일, 캡처 상태, 카운터), 탭 바(Packets, Flows, Stats, Alerts, Stream, Settings), 탭 본문, 하단 칩과 안내 줄이다.
    - Packets 탭에는 필터 입력줄, 옆에 상세 카드가 붙은 패킷 목록(Time, Dir, Process, Host, IP, Proto, Len, Info), hex 카드, 처리량 줄이 있다.
    - `1`부터 `6` 또는 `Tab`으로 탭을 바꾸고, `v`로 출력 레벨(`debug`, `info`, `warn`, `error`)을 올리면 경고와 오류만 목록에 남는다.
    - 앱 바는 `LIVE`, `BROWSE`, `PAUSED` 중 하나와 패킷·유실 카운터, 갱신 신선도 점을 보여 준다.
    - `/`로 필터, `Space`로 일시 정지, `w`로 보이는 패킷을 로그 파일로 저장, `L`로 표시 언어 전환, `?`로 전체 키 보기, `q`로 종료한다. 전체 키 목록은 키 조작, 화면 동작은 터미널 UI를 참고한다.
4. **권한 없이 해 보기**
    - 저장된 캡처 파일을 연다. 상승된 권한이 필요 없다.

```bash
network-monitor read capture.pcap
```

### 5. Egress 모니터

1. **데몬 시작**
    - Unix 전용이다. `monitor run`은 명시적인 `-i`와 `capture`와 같은 캡처 권한(Linux는 root)이 필요하며, `Ctrl-C` 또는 `monitor stop`까지 foreground로 실행된다.
    - `--demo`는 학습 구간을 5초로 줄여 경보 signal이 빠르게 발생하게 한다. 없으면 baseline이 7일 동안 학습하며 그동안 경보를 내지 않는다.

```bash
sudo network-monitor monitor run -i en0 --demo
```

2. **inspector 연결**
    - 두 번째 터미널에서 실행하며 로컬 Unix 소켓으로 연결한다.

```bash
network-monitor inspect
network-monitor monitor status
network-monitor monitor stop
```

3. **같은 상태 디렉터리**
    - 데몬, `inspect`, `monitor status/stop`은 상태 디렉터리(그 안의 `network-monitor.sock`)로 서로를 찾는다.
    - `sudo` 아래에서는 기본 상태 디렉터리가 본인의 것과 다를 수 있으므로, 모든 명령에 같은 `--state-dir <dir>`를 넘기는 것이 확실하다.
4. **선택 사항인 오프라인 국가/ASN 데이터**
    - DB-IP Lite 같은 MaxMind DB(`.mmdb`) 형식의 국가 및 ASN 데이터베이스를 내려받아 `--geoip-country-db`와 `--geoip-asn-db`로 지정한다.
    - db-ip.com 데이터는 CC-BY-4.0이며 출처 표기가 필요하다: © [db-ip.com](https://db-ip.com). MaxMind GeoLite2 파일도 동작하며, 어느 것도 번들되거나 자동으로 내려받지 않는다.
    - 데이터베이스가 없거나 파일을 열지 못하면 국가와 ASN은 unknown으로 남고 모니터는 계속 동작한다.

```bash
sudo network-monitor monitor run -i en0 \
  --geoip-country-db /path/to/dbip-country-lite.mmdb \
  --geoip-asn-db /path/to/dbip-asn-lite.mmdb
```

---

## 기능

1. **패킷 분석기**
    - 임의 인터페이스의 BPF 필터 라이브 캡처, PCAP 또는 PCAPNG 저장(`-w`, `--pcapng`)
    - 디코더: Ethernet, ARP, IPv4/IPv6, TCP, UDP, ICMP/ICMPv6, DNS(질문과 응답), TLS(SNI와 JA3/JA4 fingerprint), HTTP/1.1, 평문 HTTP/2(HPACK), QUIC long header 감지
    - TCP 스트림 재조립과 RTT 추정, `f`로 선택한 TCP 패킷의 스트림을 Stream 탭에서 따라가기
    - 기본 순서의 여섯 탭(`1`부터 `6`): Packets(패킷 목록, 상세 트리, hex 덤프, 필터 입력줄, 처리량 줄), Flows(연결 타임라인 위의 flows), Stats(프로토콜 분포 옆의 top talkers), Alerts(북마크 위의 위협 경보 표), Stream(`f`로 연 TCP 스트림의 follow-stream), Settings(출력 레벨과 언어). 탭 순서는 바꿀 수 있다.
    - 정규식 검색(`Ctrl-F`), 북마크(`m`), 인바운드/아웃바운드 보기(`i`, `o`), 열 표시 설정(`c`), 정렬 가능한 표와 폭을 끌어 조절하는 열(`s`, `S`, 머리글 클릭)
    - 마우스, 표시 언어(`L`), 카드 배치, 저장되는 레이아웃: 터미널 UI를 참고한다.
    - 패킷 목록의 평문 경고: HTTP Basic 자격 증명, FTP `PASS`, 비정상 포트의 평문 HTTP
    - 패킷에 태그를 붙이고 `threat` 필터와 일치하는 라이브 정찰 휴리스틱: `PORT-SCAN`(10초 안에 한 호스트의 서로 다른 포트 15개 이상), `HOST-SWEEP`(10초 안에 서로 다른 호스트 12개 이상), `RARE-DEST`(공인 호스트의 비일반 포트로 향하는 연결 시도나 평문 데이터이며, 출발지, 목적지, 목적지 포트마다 60초에 한 번만 태그한다. 로컬 주소로 들어오는 트래픽, 일반 서비스 포트에서 임시 포트로 가는 응답, 데이터 없는 세그먼트, TLS 레코드나 QUIC 같은 암호화 페이로드에는 붙이지 않는다)
    - 출력 레벨(`v`, Settings 탭, 또는 `[display] output_level`): `debug`(기본값, 모두 표시), `info`, `warn`, `error`. `error`는 `PORT-SCAN`과 `HOST-SWEEP`, `warn`은 `RARE-DEST`, 경고 수준 이상 징후, 재전송, `info`는 정보 수준 이상 징후, `debug`는 나머지 전부다. 레벨보다 낮은 항목은 패킷 목록, 경보 표, `w` 로그 내보내기에서 숨기며, pcap `-w` 출력, `--json`, 데몬, Flows와 Stats 탭은 걸러내지 않는다. warn 행은 차분한 전경색, error 행은 배경 채움 없는 붉은 전경색을 쓴다.
    - 패킷 수(`-c`)와 시간(`-G`) 제한을 줄 수 있는 JSON lines 출력(`--json`)
2. **Egress 모니터**
    - 백그라운드 데몬이 지속적으로 캡처하며, 각 flow의 5-tuple을 OS 소켓 테이블로 프로세스에 매핑하고, 목적지 이름을 DNS 응답과 TLS SNI에서 패시브하게 해석한다(복호화 없음).
    - 국가와 ASN은 로컬 `.mmdb` 파일에서 읽고, 상태는 로컬 SQLite 데이터베이스에 저장한다.
    - 데스크톱 알림은 macOS에서 `osascript`, Linux에서 `notify-send`를 쓰며, `--no-notify`를 주면 stderr 로그만 남긴다.
3. **다섯 가지 경보 signal**
    - 새 프로세스의 목적지: 프로세스가 한 번도 접촉한 적 없는 도메인이나 조직에 접속(severity notice)
    - 새 프로세스의 첫 외부 접속: 바이너리의 첫 outbound 연결(warning)
    - 새 국가 또는 ASN: 새로운 국가나 네트워크의 호스트와 첫 접촉(notice)
    - 송신량 급증: 60초 구간당 outbound 바이트가 프로세스의 baseline을 크게 넘는 경우이며, 8개 샘플 이후 평균보다 표준편차 4배 이상, 5 MiB 이상일 때 발생(warning, 평균의 10배 이상이면 critical)
    - 프로그램 변조: 프로세스의 바이너리 identity가 baseline 이후 바뀐 뒤 외부로 접속(critical)
    - 바이너리 identity는 macOS에서는 코드 서명(team 또는 signing identity 변경, valid에서 invalid로의 후퇴는 경보 대상이고, 같은 identity로 재서명한 업데이트는 아님)이며, 그 외에는 실행 파일의 SHA-256이다.
    - 학습 구간에는 경보를 내지 않으며, 반복되는 같은 경보는 한 시간 동안 억제된다.
4. **Inspector**
    - 데몬 소켓으로 연결하는 두 번째 TUI이며 Connections, Processes, Domains, Alerts, History 다섯 보기를 가진다.
    - `inspect --json`은 스냅샷 하나(status, 라이브 connections, 최근 경보 최대 200개)를 출력하고 종료한다.
5. **모니터 전용**
    - 관찰하고 경보만 낸다. 트래픽을 차단하거나 방화벽 처리하지 않는다.

---

## 명령

1. **전역**
    - 서브커맨드 없이 `network-monitor`를 실행하면 기본 옵션으로 라이브 캡처 TUI를 시작한다.
    - `--help`와 `--version`은 clap이 제공한다.
    - 모든 서브커맨드도 `--help`를 받는다.
2. **`list-interfaces`**
    - 각 인터페이스의 이름, 설명, 주소를 출력한다.
3. **`capture`**
    - 라이브 캡처는 인터페이스를 promiscuous 모드로 연다.
    - `-i, --interface <name>`: 인터페이스이며 기본값은 `capture.default_interface`, 그다음 자동 선택이다.
    - `-f, --filter <bpf>`: libpcap이 적용하는 BPF 캡처 필터
    - `-w, --write <file>`: TUI 실행 중 패킷을 pcap 파일로 저장한다(표시된 패킷만이 아니라 수신한 모든 패킷).
    - `--pcapng`: 기본 pcap 대신 PCAPNG로 쓴다(`-w`와 함께).
    - `--json`: TUI 대신 JSON lines를 출력하며 이 모드에서는 `-w`가 쓰이지 않는다.
    - `-s, --snaplen <n>`: 패킷당 최대 바이트, 기본 65535
    - `--buffer-size <n>`: 메모리에 보관하는 패킷 수, 기본 100000, 최소 1
    - `-c, --count <n>`: n개 패킷 후 중지하며 `--json`에서 효과가 있다.
    - `-G, --duration <seconds>`: n초 후 중지하며 `--json`에서 효과가 있다.
4. **`read <file>`**
    - pcap 또는 PCAPNG 파일을 TUI로 연다.
    - `-f, --filter <bpf>`: 파일을 읽는 동안 적용하는 BPF 필터
    - `--json`: JSON lines를 출력한다.
    - `--buffer-size <n>`: 메모리에 보관하는 패킷 수, 기본 100000
5. **`diff <file_a> <file_b>`**
    - 두 캡처 파일을 패킷 내용으로 비교해 공통 개수와, A에만 있는 패킷과 B에만 있는 패킷을 각각 최대 20개까지 출력한다.
6. **`monitor run`** (Unix 전용)
    - `-i, --interface <name>`: 필수
    - `-f, --filter <bpf>`, `--snaplen <n>`(기본 65535)
    - `--state-dir <dir>`: 런타임 상태 디렉터리(데이터베이스, 소켓, pid 파일, 로그)
    - `--geoip-country-db <file>`, `--geoip-asn-db <file>`: 오프라인 `.mmdb` 데이터베이스
    - `--demo`: 5초 학습 구간
    - `--daemonize`: 백그라운드로 분리(double fork)하며 출력은 상태 디렉터리의 `monitor.log`로 간다.
    - `--no-notify`: 데스크톱 알림을 끄고 로그만 남긴다.
7. **`monitor status`**와 **`monitor stop`** (Unix 전용)
    - `monitor status [--state-dir <dir>] [--json]`: baseline 상태, pid, 인터페이스, 가동 시간, 프로세스·목적지·경보 개수
    - 상태에는 `build_id`(기동 시점 데몬 exe 의 크기와 수정 시각), `exe_path`(그 exe 의 경로), `run_args`(데몬 자신의 실행 인자)도 들어 있어 `inspect`가 `build_id`를 지금 `exe_path`에 있는 파일과 비교하며, 이 필드가 없는 이전 빌드 데몬은 불일치로 표시하지 않는다.
    - `monitor stop [--state-dir <dir>]`: 실행 중인 데몬에 정상 종료를 요청한다.
8. **`inspect`** (Unix 전용)
    - `--state-dir <dir>`: 데몬 소켓을 찾는 데 쓰는 상태 디렉터리
    - `--socket <path>`: 이 소켓 경로를 직접 사용한다.
    - `--json`: status, 라이브 connections, 최근 경보를 JSON으로 출력하고 종료한다.

```bash
sudo network-monitor capture -i eth0 -f "tcp port 443"
sudo network-monitor capture -i en0 -w out.pcapng --pcapng
sudo network-monitor capture -i en0 --json -c 100
network-monitor read capture.pcap -f "port 53"
network-monitor diff a.pcap b.pcap
```

---

## 디스플레이 필터

1. **사용 위치**
    - TUI에서 `/`를 누르고 필터를 입력한 뒤 `Enter`를 누른다. 빈 필터는 필터를 해제한다.
    - `:name`은 `config.toml`의 저장 필터를 적용한다.
    - `-f`로 주는 BPF 필터와는 별개이며, BPF 필터는 디코딩 전에 libpcap이 적용한다.
2. **프로토콜 atom**
    - `eth`, `arp`, `ip`, `ipv4`, `ipv6`, `tcp`, `udp`, `icmp`, `icmpv6`, `dns`, `tls`
    - `threat`(또는 `alert`)는 라이브 정찰 휴리스틱이 표시한 패킷과 일치한다.
3. **필드**
    - `ip.src`, `ip.dst`: IPv4 또는 IPv6 주소
    - `tcp.port`, `udp.port`: 출발지 또는 목적지 포트 중 어느 쪽이든 일치한다. `tcp.srcport`, `tcp.dstport`, `udp.srcport`, `udp.dstport`는 한쪽만 일치한다.
    - `dns.qname`: 첫 번째 DNS 질문 이름, `tls.sni`: TLS ClientHello의 서버 이름
    - `icmp.type`, `icmp.code`, `arp.sender.ip`, `arp.target.ip`
4. **연산자**
    - 비교: `==`와 `!=`만 지원한다(순서 연산자 없음). `tcp.port != 80`은 양쪽 모두 80번 포트를 쓰지 않는다는 뜻이다.
    - 포함: `<field> contains <text>`는 `tls.sni`, `dns.qname` 같은 문자열 필드에 대한 대소문자 구분 부분 문자열 검사다.
    - 불리언: `and` / `&&`, `or` / `||`, `not` / `!`, 괄호. `and`가 `or`보다 먼저 묶인다.
5. **자유 텍스트 대체**
    - 완전한 구조화 필터가 아닌 입력은 패킷 요약(info, source, destination, protocol, length)에 대한 대소문자 무시 텍스트 일치로 바뀐다. 따라서 `tpc.port == 443` 같은 오타는 오류를 내지 않고 텍스트로 필터링한다.
6. **예시**

```markdown
tcp.port == 443 and ip.src == 10.0.0.1
(tcp || udp) && not dns
tls.sni contains example.com
dns.qname contains example
icmp.type == 8
threat
google
```

7. **Wireshark와의 관계**
    - `ip.src`, `tcp.port` 같은 일부 필드 이름은 Wireshark의 명명을 따르지만, 이것은 Wireshark 디스플레이 필터 문법이 아니라 훨씬 작은 언어다.

---

## 설정

1. **우선순위**
    - 명령줄 옵션, 그다음 `config.toml`, 그다음 내장 동작(자동 선택된 인터페이스)이다.
2. **`config.toml` 키**
    - 파일: `~/.config/network-monitor/config.toml`이며, 모든 키를 주석과 함께 담은 사본은 [docs/config.example.toml](docs/config.example.toml)이다.
    - `[capture] default_interface`: `-i`를 생략했을 때 쓰는 인터페이스
    - `[display] color_scheme`: 읽어 들이며 기본값은 `"dark"`이지만 아직 적용되지 않는다.
    - `[display] output_level`: 캡처 TUI의 최소 출력 레벨이며 `"debug"`(기본값), `"info"`, `"warn"`, `"error"` 중 하나다. 알 수 없는 값은 stderr에 경고를 한 번 출력하고 `debug`로 대체한다. `v` 키와 Settings 탭이 이 값을 덮어쓰며 선택은 `tui.toml`에 저장된다.
    - `[filters]`: `name = "filter"` 표이며 필터 입력줄에서 `:name`으로 불러온다.
    - 파싱에 실패한 파일은 stderr에 메시지(`network-monitor: ignoring invalid <path>: ...`)를 남기고 무시하며 기본값을 쓴다.
3. **사용자별 파일 전체를 담는 폴더 하나**
    - 폴더: 모든 OS에서 `~/.config/network-monitor/`(Windows에서는 `%USERPROFILE%\.config\network-monitor\`)이며, `USERPROFILE`, `HOME` 중 비어 있지 않은 첫 값 아래에 있다.
    - 홈 디렉터리가 없으면 공용 대체 폴더는 없다. TUI는 stderr에 한 줄을 출력하고 상태와 로그 저장을 끄며, `monitor` 명령은 `--state-dir`이 필요하다.
    - 내부 파일: `config.toml`, `tui.toml`(TUI 상태), `network-monitor.sock`(IPC 소켓), `network-monitor.db`(SQLite 데이터베이스), `network-monitor.pid`, `monitor.log`(`--daemonize` 후 데몬 출력), 저장된 `network-monitor-<YYYYMMDD-HHMMSS>.log` 파일
    - `--state-dir <dir>`는 데몬 파일(소켓, 데이터베이스, pid 파일, `monitor.log`)을 `<dir>`로 옮기며, `config.toml`과 저장 캡처 로그는 위 폴더에 그대로 둔다.
    - 데이터베이스에는 프로세스 이름, 실행 파일 경로, 목적지 이름과 IP, 경보가 들어 있다. 폴더를 비공개로 유지하고 공유하지 않는다.
4. **저장 캡처 로그**
    - TUI에서 `w`를 누르면 보이는 패킷이 백그라운드 스레드에서 `~/.config/network-monitor/`의 `network-monitor-<YYYYMMDD-HHMMSS>.log`(UTC 시각)로 저장되며, 끝나면 알림에 전체 경로가 나온다.
    - `w`는 예전 `s` 키를 대신한다. 이제 `s`는 포커스된 표를 정렬한다.
    - root(유효 사용자 ID 0)로 실행 중이면 다른 사용자가 소유한 심볼릭 링크를 통한 쓰기를 거부하고 오류 알림을 표시한다.
5. **디버그 추적**
    - `NETWORK_MONITOR_DEBUG=1`(`0`이 아닌 비어 있지 않은 값)은 실제 적용된 파라미터와 IPC 트래픽을 stderr에 출력한다.

---

## 키 조작

1. **캡처 TUI: 이동**
    - `j` / `Down`, `k` / `Up`: 포커스된 카드의 선택 행 이동. 상세 카드와 hex 카드는 스크롤한다.
    - `g` / `Home`: 첫 행, `G` / `End`: 마지막 행이며 `LIVE`로 복귀
    - `PageUp` / `PageDown`: 한 화면만큼 행 이동
    - 위로 이동하거나 맨 위로 가면 `LIVE`에서 `BROWSE`가 되고, `G`나 마지막 행 도달로 `LIVE`에 복귀한다.
2. **캡처 TUI: 정렬**
    - `s`: 포커스된 표의 다음 열로 정렬하며, 순환의 끝은 캡처 순서다.
    - `S`: 활성 열의 방향을 뒤집고 그다음 캡처 순서로 되돌린다.
    - 둘 다 머리글 클릭과 같은 세 단계를 따르며, 활성 정렬은 카드 제목에 표시된다(`정렬 길이↓`).
3. **캡처 TUI: 필터와 검색**
    - `/`: 필터 입력줄(`Enter` 적용, `Esc` 취소, `:name`은 저장 필터 호출)
    - `Ctrl-F`: info, source, destination에 대한 정규식 검색(`Enter` 실행, `Esc` 해제), `n`은 다음 일치로 이동
    - `i` / `o`: 인바운드만 / 아웃바운드만 표시, 다시 누르면 해제
4. **캡처 TUI: 보기**
    - `1`부터 `6`까지 탭 열기, `Tab` / `Right` 다음 탭, `Shift-Tab` / `Left` 이전 탭, `Enter` 다음 카드로 포커스 이동(Settings에서는 선택한 설정 변경)
    - `t` / `P` Stats, `F` / `T` Flows, `'` Alerts, `f` 선택한 TCP 패킷의 스트림을 Stream 탭에서 열기, `Esc` Packets로 복귀
    - `<` / `>`: 활성 탭을 왼쪽 또는 오른쪽으로 이동, 탭 바에서 탭을 끌면 순서가 바뀐다(`Esc`로 취소).
    - `d`는 상세 카드, `x`는 hex 카드, `z`는 둘 다 토글한다.
    - `c`는 열 편집기를 연다. `1`부터 `8`까지 Time, Dir, Process, Host, IP, Proto, Len, Info를 토글하고 `Esc`, `Enter`, `c`로 닫는다.
5. **캡처 TUI: 동작**
    - `Space`: 일시 정지 또는 재개
    - `m`: 선택한 패킷에 북마크
    - `w`: 보이는 패킷을 로그 파일로 저장
    - `v`: 출력 레벨 순환(`debug`, `info`, `warn`, `error`)
    - `Shift`와 방향키: 포커스된 카드 이동(터미널 UI 참고)
    - `l`: 카드 순서, 크기, 열 폭 초기화, `L`: 표시 언어 전환
    - `?`: 도움말, `q`, `Ctrl-C`: 종료, `Esc`: Packets로 복귀하며 Packets에서는 종료
6. **Inspector**
    - `Tab` / `Right` 다음 보기, `Shift-Tab` / `Left` 이전 보기, `1`부터 `5`까지 Connections, Processes, Domains, Alerts, History로 이동
    - `j` / `Down`, `k` / `Up`, `PageUp` / `PageDown`, `Home` / `End`: 선택 행 이동
    - `Enter`: 표 카드와 상세 카드 사이로 포커스 이동
    - `s` / `S`: 포커스된 표 정렬이며 캡처 TUI와 같은 순환이다.
    - `/`: Connections 보기 검색(`Enter`로 유지, `Esc`로 해제)
    - `r`: status와 connections 새로 고침
    - `Shift`와 위 또는 아래 방향키: 포커스된 카드 이동, `l`: 현재 보기의 배치 초기화, `L`: 표시 언어 전환
    - `?`: 도움말, `q`, `Esc`, `Ctrl-C`: 종료
    - `!`: 강제 재시작, 데몬이 실행 중인 자기 exe 가 디스크에서 다른 빌드로 바뀐 뒤에만 동작한다(2초마다 확인). 데몬을 종료하고 같은 경로의 exe 로 데몬 자신의 `monitor run` 인자에 `--daemonize`를 붙여 다시 띄운 뒤 `inspect`를 다시 실행한다. 먼저 캡처 권한(이 `inspect`와, 숨은 `monitor preflight` 명령으로 실행한 새 exe 양쪽), 상태 폴더 쓰기 권한, 데몬 인자의 절대 경로, exe 소유자를 확인하며, 하나라도 실패하면 오류 알림만 띄우고 데몬은 그대로 둔다.
    - 데몬의 exe 가 디스크에서 다른 빌드이면 앱 바에 `● 빌드 불일치`가 표시되고 경고색 `[!] 강제 재시작` 칩이 하단 맨 앞에 나타나며, 클릭하면 `!`와 같게 동작한다.

---

## 터미널 UI

1. **모양**
    - 두 TUI가 같은 스타일을 쓴다. 앱 바(제품 배지, 인터페이스 또는 파일, 캡처 상태, 신선도 점), 카드, 도움말과 종료를 오른쪽 끝에 고정한 클릭 가능한 하단 칩, 안내 줄로 구성된다.
    - 캡처 화면은 앱 바, 탭 바, 활성 탭의 본문, 처리량 줄(Packets 탭에서만, 화면 높이가 24행 이상일 때), 두 줄의 하단(칩과 안내)을 위에서 아래로 쌓으며, `[v] <level>` 칩이 언어 칩 앞에 놓인다.
    - 카드는 둥근 테두리이며, 포인터 아래의 카드는 더 밝은 테두리, 포커스된 카드는 제목이 반전된 굵은 테두리가 된다.
    - 표는 별도의 회색 거터 열에 스크롤바(`░` 트랙, `█` 썸)를 보이고 아래 테두리에 ` a-b / total `을 표시하며, 선택 행은 색 띠로 나타난다.
    - 안내 줄은 포인터 아래 대상, 그다음 포커스된 카드에 해당하는 키를 보여 준다. 알림은 4초 동안 유지되며 오류 알림만 붉은 채움이다.
    - `?`는 도움말 오버레이(이중선 테두리, 어두운 배경, 키보드와 마우스 구역)를 열며, 아무 키나 클릭으로 닫는다. 이중선은 오버레이에만 나타난다.
    - 한글 외에는 ASCII, 상자 그리기 문자, `·`, `…`, `●`/`○`, `↑`/`↓`만 그리므로 어떤 터미널 글꼴에서도 칸 폭이 유지된다. 패킷 상세 트리는 `>`, 스트림 따라가기는 클라이언트 데이터를 `>>`, 서버 데이터를 `<<`로 표시하며, 패킷 요약의 화살표는 `->`로 그린다.
    - 터미널이 60x16보다 작으면 경고 한 줄만 표시한다.
2. **마우스**
    - 카드를 클릭하면 포커스되고 행을 클릭하면 선택되며, 휠은 포인터 아래 카드를 스크롤한다.
    - 열 머리글을 클릭하면 정렬한다. 숫자와 시간은 내림차순, 문자는 오름차순으로 시작하고 세 번째 클릭은 캡처 순서로 되돌린다.
    - 두 열 머리글 사이 경계를 끌면 열 폭이 바뀌고, 경계를 더블클릭하면 자동 폭으로 돌아간다.
    - 카드 사이 경계를 끌면 카드 크기가 바뀐다. 스크롤바 썸을 끌거나 트랙을 클릭해 한 화면씩 이동한다.
    - 하단 칩, 탭, 언어 칩은 클릭할 수 있으며 해당 키의 동작을 실행한다.
3. **배치**
    - 카드 제목에서 왼쪽 버튼을 400 ms 누른 뒤 끌면 그 그룹의 카드 순서를 바꾼다. 순서는 실시간으로 미리 보이고, 놓으면 확정되며, `Esc`는 취소한다.
    - 이동 중인 카드는 강조색의 가는 이중 점선 테두리를 가진다.
    - 터미널이 알려 주지 않은 버튼 뗌은 다음 마우스 이동이나 누름에서 뗀 것으로 처리한다. 카드 이동, 경계 또는 열 경계 드래그, 스크롤바 드래그가 모두 끝나며 배치가 바뀌었으면 저장한다. 인스펙터 보기를 전환해도 드래그가 같은 방식으로 끝난다.
    - `Shift`와 방향키는 포커스된 카드를 한 칸 옮기고, `l`은 현재 화면의 순서, 크기, 열 폭을 되돌리며 언어는 유지한다.
    - 칸 크기는 칸에 남는다. 카드를 옮기면 위치만 바뀌고 크기는 바뀌지 않는다.
4. **표시 언어**
    - 모든 화면 문구는 영어와 한국어가 함께 있는 하나의 카탈로그에서 나오며 기본은 영어다.
    - `L`(칩 `[L] EN` 또는 `[L] KO`)은 재시작 없이 즉시 언어를 바꾸고 선택은 저장된다.
    - 패킷 데이터, 주소, 프로토콜 이름, 프로세스 이름, 필터 텍스트는 번역하지 않는다. 상세 카드의 프로토콜 필드 레이블은 프로토콜 표기 그대로 둔다.
5. **상태 파일**
    - 경로: `~/.config/network-monitor/tui.toml`이며 `config.toml` 옆에 있고 두 TUI가 함께 읽고 쓴다.
    - 내용: `version`, `language`, `output_level`, `capture_tab_order`, 카드 순서, 분할 비율, 열 폭이다. 기본값과 다른 값만 기록하며(`output_level`은 선택한 뒤에만), 변경이 확정될 때만(놓기, 드래그 종료, 초기화, 언어 전환, 출력 레벨 변경, 탭 이동) 저장한다.
    - 저장은 백그라운드 쓰기 스레드에서 실행된다. 연속된 변경은 가장 최근 상태 하나로 합치고, 저장에 실패하면 오류 알림을 표시하며, 종료할 때는 쓰기 스레드를 최대 5초 기다리고 마지막 저장이 실패했거나 아직 진행 중이면 stderr에 경고 한 줄을 출력한다.
    - 같은 폴더의 임시 파일을 거쳐 교체하므로 저장이 중단돼도 기존 파일이 남고, 심볼릭 링크(최대 16단계 연쇄)는 유지한 채 최종 대상 파일을 갱신하며, root로 실행 중이면 다른 사용자가 소유한 링크는 거부한다.
    - 중단된 저장이 남긴 임시 파일은 시작할 때 만든 지 60초 이상 지난 것만 지우므로, 실행 중인 다른 인스턴스의 저장을 방해하지 않는다.
    - 읽을 수 없거나 다른 버전인 파일은 첫 저장이 덮어쓰기 전에 `tui.toml.bak`으로 복사한다. 직접 고친 파일의 잘못된 항목은 건너뛰고, 열 폭은 512칸으로 제한한다.
    - 파일을 지우면 기본 배치와 영어로 돌아간다.

---

## 문제 해결

1. **Linux에서 `Capture needs root or the CAP_NET_RAW capability`**
    - `sudo`로 실행하거나, capability를 한 번 부여해 일반 사용자로 캡처한다: `sudo setcap cap_net_raw,cap_net_admin=eip "$(command -v network-monitor)"`.
    - 검사는 유효 사용자 ID 0 또는 프로세스 유효 capability의 `CAP_NET_RAW`를 허용한다. 바이너리를 다시 빌드하거나 설치하면 capability가 사라지므로 `setcap`을 다시 실행한다.
2. **`sudo: network-monitor: command not found`**
    - `sudo`의 `PATH`에 `~/.cargo/bin`이 없을 수 있다. `sudo env "PATH=$PATH" network-monitor`를 실행하거나 전체 경로를 쓴다.
3. **인터페이스나 패킷이 보이지 않음**
    - 장치 목록이 비어 있으면 `list-interfaces`가 `No interfaces found. You may need elevated privileges.`를 출력한다.
    - Windows에서는 Npcap x64 런타임을 설치하고 관리자 프롬프트를 쓴다.
    - `-i <name>`으로 인터페이스를 직접 지정한다. 자동 선택이 실제 트래픽이 지나가는 인터페이스와 다를 수 있다.
4. **빌드에서 libpcap을 찾지 못함**
    - Linux: `libpcap-dev`(Debian/Ubuntu) 또는 `libpcap-devel`(Fedora)을 설치한다. `LIBPCAP_LIBDIR`가 없으면 빌드는 `pkg-config`를 쓴다.
    - Windows: Npcap SDK가 링커 경로에 없는 경우다. `LIB`(또는 `LIBPCAP_LIBDIR`)를 SDK의 `Lib\x64` 폴더로 지정한다(x86 `Lib` 폴더가 아님).
5. **`the egress monitor daemon is only supported on Unix`**
    - `monitor`와 `inspect`는 Windows에서 쓸 수 없다. Windows에서는 `capture`와 `read`를 쓴다.
6. **`cannot connect to daemon at <path>`**
    - 데몬이 실행 중이 아니거나 `inspect`가 다른 상태 디렉터리를 보고 있다. `monitor run`을 시작하고 둘에 같은 `--state-dir`를 넘긴다.
7. **경보가 나타나지 않음**
    - baseline이 아직 학습 중이다. 기본 7일이며 `--demo`는 5초다. `network-monitor monitor status`로 확인한다(`learning` 또는 `active`).
    - OS 소켓 테이블이 프로세스에 귀속시킬 수 있는 outbound 연결만 프로세스별 signal을 발생시킨다. 모든 사용자에 걸친 완전한 귀속을 위해 데몬을 root로 실행한다.
8. **국가와 ASN이 unknown으로 표시됨**
    - `.mmdb` 파일을 지정하지 않았거나 파일을 열지 못했다. 모니터는 이 실패를 보고하지 않는다.
9. **필터가 기대와 다르게 동작함**
    - 완전한 구조화 식이 아닌 입력은 텍스트 검색이 된다. 디스플레이 필터 섹션과 비교한다.
10. **설정 값이 무시됨**
    - stderr에서 `network-monitor: ignoring invalid <path>`를 확인하고, 설정 섹션에서 사용 중인 OS의 파일 경로를 확인한다.
11. **배치나 언어가 복원되지 않음**
    - `tui.toml`을 읽지 못하면 알림이 파일 이름을 알려 준다. 파일은 `tui.toml.bak`으로 복사되고 기본값을 쓰며, 다음 확정 때 새 파일이 기록된다.
    - 잘못된 항목은 건너뛴다. 처음부터 다시 시작하려면 `~/.config/network-monitor/tui.toml`을 지운다.

---

## 한계

1. **모니터 전용**
    - 설계상 차단이나 방화벽 처리는 없다.
2. **프로토콜**
    - QUIC Initial 복호화는 구현되지 않았다. DNS에서 유도한 이름이 같은 목적지를 다룬다.
    - HTTP/2의 HPACK Huffman 리터럴은 풀지 않는다(HTTP/2는 대개 어차피 TLS 암호화). 사전 합의된 평문 h2c만 디코딩한다.
3. **플랫폼**
    - egress 모니터 데몬과 inspector는 Unix 전용이다.
    - Windows 프로세스 귀속은 가볍게만 테스트되었다.
4. **아직 적용되지 않음**
    - `display.color_scheme`은 파싱되지만 테마를 바꾸지 않으며(어두운 팔레트 하나만 항상 쓴다), 키 바인딩은 재매핑할 수 없다.
5. **메모리**
    - 패킷은 제한된 링에 보관된다(`--buffer-size`, 기본 100000). 오래된 패킷은 화면에서 사라진다. 모두 보존하려면 `-w`로 pcap을 쓴다.

---

## 개발

1. **검증**
    - `cargo fmt --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace`, `cargo build --workspace`
    - `monitor`, `ipc`, `inspector` 모듈은 Unix 타깃에서만 컴파일되고 실행된다.
2. **구조**
    - crate와 모듈 구성: [ARCHITECTURE-ko.md](ARCHITECTURE-ko.md)

---

## 라이선스

1. **라이선스**
    - Apache-2.0, [LICENSE.md](LICENSE.md) 참고
    - GeoIP 데이터를 내려받았다면 그 데이터는 자체 라이선스를 따른다(DB-IP Lite: CC-BY-4.0, © db-ip.com).
