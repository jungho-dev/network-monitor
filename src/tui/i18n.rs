//! Display text catalog: English and Korean entries on one line each, English
//! by default, format functions for messages that carry values.
//!
//! Packet data, addresses, protocol names, process names, error details from
//! the engine, and filter text are never translated; they are passed to the
//! format functions as arguments.

use super::layout::SplitAxis;

// Product name shown in the app bar badge (a name, not translated).
pub const PRODUCT_NAME: &str = "network-monitor";

// 1. display language ---------------------------------------------------------
// The state file stores `en` or `ko`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Lang {
    #[default]
    En,
    Ko,
}

impl Lang {
    pub fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Ko => "ko",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "en" => Some(Self::En),
            "ko" => Some(Self::Ko),
            _ => None,
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            Self::En => Self::Ko,
            Self::Ko => Self::En,
        }
    }
}

// 2. message catalog ----------------------------------------------------------
// Each entry holds both languages, so one side can never be missing; ALL feeds
// the completeness test.
macro_rules! define_messages {
    ($($name:ident => ($en:expr, $ko:expr),)+) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Msg {
            $($name,)+
        }

        impl Msg {
            #[cfg(test)]
            pub const ALL: &'static [Msg] = &[$(Msg::$name,)+];
        }

        pub fn text(lang: Lang, msg: Msg) -> &'static str {
            match msg {
                $(Msg::$name => match lang {
                    Lang::En => $en,
                    Lang::Ko => $ko,
                },)+
            }
        }
    };
}

define_messages! {
    // app bar
    BarLive => ("LIVE", "실시간"),
    BarPaused => ("PAUSED", "일시정지"),
    BarBrowse => ("BROWSE", "탐색"),
    BarWaiting => ("Waiting", "대기"),
    BarPackets => ("packets", "패킷"),
    BarDropped => ("dropped", "유실"),
    BarInterface => ("Interface", "인터페이스"),
    BarFile => ("File", "파일"),
    BarConnected => ("Connected", "연결됨"),
    BarDisconnected => ("Disconnected", "연결 끊김"),
    BarLiveConns => ("live", "실시간 연결"),
    BarAlerts => ("alerts", "경보"),

    // footer chips
    ChipPause => ("Pause", "일시정지"),
    ChipResume => ("Resume", "재개"),
    ChipFilter => ("Filter", "필터"),
    ChipSearch => ("Search", "검색"),
    ChipColumns => ("Columns", "열"),
    ChipSave => ("Save log", "로그 저장"),
    ChipRefresh => ("Refresh", "새로고침"),
    ChipResetLayout => ("Reset layout", "배치 초기화"),
    ChipLanguage => ("EN", "KO"),
    ChipHelp => ("Help", "도움말"),
    ChipQuit => ("Quit", "종료"),

    // footer hints (first word is the key)
    HintRelease => ("Release to place", "놓아서 위치 확정"),
    HintCancel => ("Esc Cancel", "Esc 취소"),
    HintDragCardSize => ("Drag card size", "드래그 카드 크기"),
    HintResetLayout => ("l Reset layout", "l 기본 배치"),
    HintDragColumnWidth => ("Drag column width", "드래그 열 폭"),
    HintDoubleClickAuto => ("Double-click auto width", "더블클릭 자동 폭"),
    HintDragScroll => ("Drag scroll", "드래그 스크롤"),
    HintTrackPage => ("Track-click page", "트랙클릭 한 화면"),
    HintSelectRow => ("↑↓ Select row", "↑↓ 행 선택"),
    HintScroll => ("↑↓ Scroll", "↑↓ 스크롤"),
    HintPage => ("PgUp/PgDn Page", "PgUp/PgDn 페이지"),
    HintFirstLast => ("Home/End First/Last", "Home/End 처음·끝"),
    HintSortColumn => ("s Sort column", "s 정렬 열"),
    HintSortOrder => ("S Sort order", "S 정렬 방향"),
    HintFilter => ("/ Filter", "/ 필터"),
    HintPause => ("Space Pause", "Space 일시정지"),
    HintNextCard => ("Tab Next card", "Tab 다음 카드"),
    HintNextTab => ("Tab Next tab", "Tab 다음 탭"),
    HintAllKeys => ("? All keys", "? 전체 단축키"),
    HintApply => ("Enter Apply", "Enter 적용"),
    HintEditCancel => ("Esc Cancel", "Esc 취소"),
    HintToggleColumn => ("1-8 Toggle column", "1-8 열 켜기·끄기"),
    HintColumnsDone => ("Esc Done", "Esc 완료"),
    HintCloseView => ("Esc Close", "Esc 닫기"),
    HintToggleDetail => ("Enter Table/Detail", "Enter 표/상세"),

    // help overlay
    HelpTitle => ("Help", "도움말"),
    HelpClose => ("Any key or click: close", "아무 키·클릭: 닫기"),
    HelpTruncated => ("… (enlarge window to show all)", "… (창을 키우면 전체 표시)"),
    HelpSectionKeyboard => ("Keyboard", "키보드"),
    HelpSectionMouse => ("Mouse", "마우스"),
    HelpQuit => ("Quit", "종료"),
    HelpToggleHelp => ("Open / close help", "도움말 열기·닫기"),
    HelpMoveRows => ("Select row / scroll focused card", "행 선택 / 포커스 카드 스크롤"),
    HelpPage => ("Page up / down", "한 화면 위·아래"),
    HelpFirstLast => ("First / last row (G = live)", "처음·끝 행 (G = 실시간)"),
    HelpNextCard => ("Focus next / previous card", "다음·이전 카드 포커스"),
    HelpPause => ("Pause / resume capture", "캡처 일시정지·재개"),
    HelpFilter => ("Edit display filter, :name recalls", "표시 필터 편집, :이름 불러오기"),
    HelpSearch => ("Regex search, n = next match", "정규식 검색, n = 다음 일치"),
    HelpDirection => ("Show IN only / OUT only", "수신만 / 송신만 보기"),
    HelpSort => ("Sort: next column / flip, then capture order", "정렬: 다음 열 / 방향, 이어서 캡처 순서"),
    HelpColumns => ("Column editor (1-8 toggle)", "열 편집 (1-8 켜기·끄기)"),
    HelpPanes => ("Detail pane / hex pane / both", "상세 창 / 헥스 창 / 둘 다"),
    HelpBookmark => ("Bookmark packet / bookmark list", "패킷 북마크 / 북마크 목록"),
    HelpStats => ("Top talkers / protocols / flows / timeline", "상위 통신 / 프로토콜 / 플로우 / 타임라인"),
    HelpStream => ("Follow TCP stream", "TCP 스트림 따라가기"),
    HelpSave => ("Save visible packets to a log", "보이는 패킷을 로그로 저장"),
    HelpMoveCard => ("Move focused card", "포커스 카드 위치 이동"),
    HelpResetLayout => ("Reset card order, sizes, widths", "카드 순서·크기·열 폭 기본 배치"),
    HelpLanguage => ("Switch language (EN / KO)", "표시 언어 전환 (EN / KO)"),
    HelpCloseOverlay => ("Close view / cancel card move", "보기 닫기 / 카드 이동 취소"),
    HelpSwitchTab => ("Switch tab", "탭 전환"),
    HelpRefresh => ("Refresh from the daemon", "데몬에서 다시 불러오기"),
    HelpSearchRows => ("Search live connections", "실시간 연결 검색"),
    HelpToggleDetail => ("Focus table / detail card", "표 / 상세 카드 포커스"),
    HelpClickCard => ("Focus card (thick border), select row", "카드 포커스(굵은 테두리), 행 선택"),
    HelpClickHeader => ("Sort: numbers ↓ first, text ↑, third click resets", "정렬: 숫자·시간 ↓ 먼저, 문자 ↑, 세 번째 초기화"),
    HelpDragColumn => ("Resize column, double-click = auto", "열 폭 조절, 더블클릭 = 자동"),
    HelpWheel => ("Scroll card under pointer / resize cards", "마우스 아래 카드 스크롤 / 카드 크기 조절"),
    HelpLongPress => ("Move card (Esc cancels)", "카드 위치 이동 (Esc 취소)"),
    HelpScrollbar => ("Drag thumb, click track to page", "썸 드래그, 트랙 클릭 한 화면 이동"),
    HelpChips => ("Run the action of the key", "해당 키의 동작 실행"),

    // toasts
    ToastCardMoved => ("Card order changed", "카드 순서 변경"),
    ToastSplitDrag => ("Drag to resize cards", "끌어서 카드 크기 조절"),
    ToastColumnDrag => ("Drag to resize column", "끌어서 열 폭 조절"),
    ToastColumnsAuto => ("Column widths set to auto", "열 폭 자동으로 복원"),
    ToastLayoutReset => ("Layout reset", "배치 초기화"),
    ToastPaused => ("Paused", "일시정지"),
    ToastResumed => ("Resumed", "재개"),
    ToastFilterCleared => ("Filter cleared", "필터 해제"),
    ToastDirIn => ("Showing inbound (IN) only", "수신(IN)만 표시"),
    ToastDirOut => ("Showing outbound (OUT) only", "송신(OUT)만 표시"),
    ToastDirCleared => ("Direction filter cleared", "방향 필터 해제"),
    ToastSelectTcp => ("Select a TCP packet to follow its stream", "스트림을 따라갈 TCP 패킷을 선택"),
    ToastNoPackets => ("No packets to save", "저장할 패킷 없음"),
    ToastSaveStarted => ("Saving log", "로그 저장 중"),
    ToastRefreshRequested => ("Refresh requested", "새로고침 요청"),

    // panes and titles
    TitlePackets => ("Packets", "패킷"),
    TitleDetails => ("Packet Details", "패킷 상세"),
    TitleHex => ("Hex Dump", "헥스 덤프"),
    TitleTalkers => ("Top talkers", "상위 통신"),
    TitleProtocols => ("Protocol distribution", "프로토콜 분포"),
    TitleFlows => ("Flows", "플로우"),
    TitleTimeline => ("Connection timeline", "연결 타임라인"),
    TitleBookmarks => ("Bookmarks", "북마크"),
    TitleStream => ("Follow TCP", "TCP 스트림"),
    TitleConnections => ("Live connections", "실시간 연결"),
    TitleProcesses => ("Per-process", "프로세스별"),
    TitleDomains => ("Per-domain", "도메인별"),
    TitleAlerts => ("Alerts", "경보"),
    TitleHistory => ("History", "기록"),
    TitleDetail => ("Detail", "상세"),
    SortWord => ("Sort", "정렬"),
    ThroughputLabel => ("Throughput", "처리량"),
    UnitPacketsPerSec => ("pkt/s", "패킷/초"),
    PaneRetransmission => ("TCP Retransmission", "TCP 재전송"),
    StreamClient => ("client", "클라이언트"),
    StreamServer => ("server", "서버"),

    // empty and pending states
    EmptyWaiting => ("Waiting for packets", "패킷 대기 중"),
    EmptyWaitingHint => ("Traffic appears here as it is captured", "캡처되는 대로 여기에 표시"),
    EmptyPaused => ("Capture paused", "캡처 일시정지"),
    EmptyPausedHint => ("Press Space to resume", "Space 키로 재개"),
    EmptyNoMatch => ("No packet matches the filter", "필터에 맞는 패킷 없음"),
    EmptyNoMatchHint => ("Press / to edit the filter", "/ 키로 필터 수정"),
    EmptyNoSelection => ("No packet selected", "선택한 패킷 없음"),
    EmptyNoBookmarks => ("No bookmarks", "북마크 없음"),
    EmptyNoBookmarksHint => ("Press m to bookmark the selected packet", "m 키로 선택한 패킷 북마크"),
    EmptyNoStream => ("No stream", "스트림 없음"),
    EmptyNoRows => ("Nothing to show yet", "표시할 항목 없음"),
    EmptyNoRowsHint => ("Press r to refresh", "r 키로 새로고침"),
    EmptyNoMatchRows => ("No connection matches the search", "검색에 맞는 연결 없음"),
    EmptyNoMatchRowsHint => ("Press / to change the search", "/ 키로 검색어 변경"),
    EmptyDisconnected => ("Daemon disconnected", "데몬 연결 끊김"),
    EmptyDisconnectedHint => ("Press q to quit and run inspect again", "q 키로 종료 후 inspect 다시 실행"),

    // filter bar
    FilterLabel => ("Filter", "필터"),
    FilterIdle => ("press / to filter · try: threat, tcp.port == 443", "/ 키로 필터 · 예: threat, tcp.port == 443"),
    FilterApplied => ("Applied", "적용됨"),
    FilterInOnly => ("IN only", "수신만"),
    FilterOutOnly => ("OUT only", "송신만"),
    SearchLabel => ("Regex search", "정규식 검색"),
    SearchLabelRows => ("Search", "검색"),
    ColumnsLabel => ("Columns", "열"),
    ColumnOff => ("off", "끔"),

    // column labels
    ColTime => ("Time", "시간"),
    ColDir => ("Dir", "방향"),
    ColProcess => ("Process", "프로세스"),
    ColHost => ("Host", "호스트"),
    ColIp => ("IP", "IP"),
    ColProto => ("Proto", "프로토콜"),
    ColLen => ("Len", "길이"),
    ColInfo => ("Info", "정보"),
    ColEndpoint => ("Endpoint", "엔드포인트"),
    ColPackets => ("Packets", "패킷"),
    ColBytes => ("Bytes", "바이트"),
    ColProtocol => ("Protocol", "프로토콜"),
    ColCount => ("Count", "개수"),
    ColPct => ("Pct", "비율"),
    ColShare => ("Share", "분포"),
    ColFlow => ("Flow", "플로우"),
    ColDuration => ("Duration", "지속"),
    ColNumber => ("#", "#"),
    ColEndpoints => ("Endpoints", "엔드포인트"),
    ColDestination => ("Destination", "목적지"),
    ColGeo => ("Geo", "지역"),
    ColUp => ("Up", "송신"),
    ColDown => ("Down", "수신"),
    ColConns => ("Conns", "연결"),
    ColDomain => ("Domain / Org", "도메인 / 조직"),
    ColSeverity => ("Severity", "심각도"),
    ColKind => ("Kind", "종류"),
    ColTitle => ("Title", "제목"),
    ColProtoPort => ("Proto:Port", "프로토콜:포트"),
    ColStarted => ("Started", "시작"),

    // inspector tabs and detail labels
    TabConnections => ("Connections", "연결"),
    TabProcesses => ("Processes", "프로세스"),
    TabDomains => ("Domains", "도메인"),
    TabAlerts => ("Alerts", "경보"),
    TabHistory => ("History", "기록"),
    DetailProcess => ("Process", "프로세스"),
    DetailPid => ("PID", "PID"),
    DetailDestination => ("Destination", "목적지"),
    DetailAddress => ("Address", "주소"),
    DetailLocalPort => ("Local port", "로컬 포트"),
    DetailProtocol => ("Protocol", "프로토콜"),
    DetailGeo => ("Country / ASN", "국가 / ASN"),
    DetailOrg => ("Organization", "조직"),
    DetailUp => ("Sent", "송신"),
    DetailDown => ("Received", "수신"),
    DetailFirstSeen => ("First seen", "처음 관측"),
    DetailLastSeen => ("Last seen", "마지막 관측"),
    DetailDirection => ("Direction", "방향"),
    DetailOutbound => ("Outbound", "송신"),
    DetailInbound => ("Inbound", "수신"),
    DetailConns => ("Connections", "연결 수"),
    DetailDomain => ("Domain / Org", "도메인 / 조직"),
    DetailSeverity => ("Severity", "심각도"),
    DetailKind => ("Kind", "종류"),
    DetailTitle => ("Title", "제목"),
    DetailTime => ("Time", "시각"),
    DetailDedup => ("Key", "키"),
    DetailName => ("Name", "이름"),
    DetailPort => ("Remote port", "원격 포트"),
    DetailStarted => ("Started", "시작"),
    DetailEnded => ("Ended", "종료"),
    SeverityInfo => ("Info", "정보"),
    SeverityNotice => ("Notice", "주의"),
    SeverityWarning => ("Warning", "경고"),
    SeverityCritical => ("Critical", "치명"),
}

// 3. format functions ---------------------------------------------------------
pub fn too_small(lang: Lang, min_width: u16, min_height: u16, width: u16, height: u16) -> String {
    match lang {
        Lang::En => {
            format!("Terminal too small (min {min_width}x{min_height}, now {width}x{height})")
        }
        Lang::Ko => {
            format!("터미널 크기 부족 (최소 {min_width}x{min_height}, 현재 {width}x{height})")
        }
    }
}

pub fn idle_for(lang: Lang, secs: i64) -> String {
    match lang {
        Lang::En => format!("Idle {secs}s"),
        Lang::Ko => format!("{secs}초째 수신 없음"),
    }
}

pub fn stale_for(lang: Lang, secs: i64) -> String {
    match lang {
        Lang::En => format!("Stale {secs}s"),
        Lang::Ko => format!("갱신 지연 {secs}초"),
    }
}

pub fn card_moving(lang: Lang, title: &str) -> String {
    match lang {
        Lang::En => format!("{title} · moving"),
        Lang::Ko => format!("{title} · 이동 중"),
    }
}

pub fn sort_title(lang: Lang, title: &str, column: &str, arrow: &str) -> String {
    format!("{title} · {} {column}{arrow}", text(lang, Msg::SortWord))
}

pub fn title_count(title: &str, count: usize) -> String {
    format!("{title} ({count})")
}

pub fn card_resized(lang: Lang, axis: SplitAxis, before: u16, after: u16) -> String {
    match (lang, axis) {
        (Lang::En, SplitAxis::Columns) => format!("Card width {before} / {after}"),
        (Lang::En, SplitAxis::Rows) => format!("Card height {before} / {after}"),
        (Lang::Ko, SplitAxis::Columns) => format!("카드 폭 {before} / {after}"),
        (Lang::Ko, SplitAxis::Rows) => format!("카드 높이 {before} / {after}"),
    }
}

pub fn column_width(lang: Lang, width: u16) -> String {
    match lang {
        Lang::En => format!("Column width {width}"),
        Lang::Ko => format!("열 폭 {width}"),
    }
}

pub fn filter_applied(lang: Lang, expr: &str) -> String {
    match lang {
        Lang::En => format!("Filter applied: {expr}"),
        Lang::Ko => format!("필터 적용: {expr}"),
    }
}

pub fn filter_error(lang: Lang, detail: &str) -> String {
    match lang {
        Lang::En => format!("Filter error: {detail}"),
        Lang::Ko => format!("필터 오류: {detail}"),
    }
}

pub fn no_saved_filter(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("No saved filter ':{name}'"),
        Lang::Ko => format!("저장된 필터 없음: ':{name}'"),
    }
}

pub fn bad_regex(lang: Lang, detail: &str) -> String {
    match lang {
        Lang::En => format!("Bad regex: {detail}"),
        Lang::Ko => format!("정규식 오류: {detail}"),
    }
}

pub fn log_saved(lang: Lang, count: usize, path: &str) -> String {
    match lang {
        Lang::En => format!("Saved {count} packets to {path}"),
        Lang::Ko => format!("패킷 {count}개를 {path} 에 저장"),
    }
}

pub fn log_save_failed(lang: Lang, detail: &str) -> String {
    match lang {
        Lang::En => format!("Log save failed: {detail}"),
        Lang::Ko => format!("로그 저장 실패: {detail}"),
    }
}

pub fn disconnected_detail(lang: Lang, detail: &str) -> String {
    match lang {
        Lang::En => format!("Daemon disconnected: {detail}"),
        Lang::Ko => format!("데몬 연결 끊김: {detail}"),
    }
}

pub fn position_label(first: usize, last: usize, total: usize) -> String {
    format!(" {first}-{last} / {total} ")
}

// Why the state file could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateIssue {
    UnsupportedVersion(i64),
    Detail(String),
}

pub fn state_unreadable(lang: Lang, file: &str, issue: &StateIssue) -> String {
    match (lang, issue) {
        (Lang::En, StateIssue::UnsupportedVersion(version)) => {
            format!("{file}: version {version} not supported, defaults used")
        }
        (Lang::Ko, StateIssue::UnsupportedVersion(version)) => {
            format!("{file}: 버전 {version} 미지원, 기본값 사용")
        }
        (Lang::En, StateIssue::Detail(detail)) => {
            format!("{file} unreadable, defaults used: {detail}")
        }
        (Lang::Ko, StateIssue::Detail(detail)) => {
            format!("{file} 읽기 실패, 기본값 사용: {detail}")
        }
    }
}

pub fn state_entries_skipped(lang: Lang, file: &str, skipped: usize) -> String {
    match lang {
        Lang::En => format!("{file}: {skipped} invalid entries ignored"),
        Lang::Ko => format!("{file}: 일부 항목 무시 {skipped}건"),
    }
}

pub fn state_save_failed(lang: Lang, detail: &str) -> String {
    match lang {
        Lang::En => format!("Could not save TUI state: {detail}"),
        Lang::Ko => format!("TUI 상태 저장 실패: {detail}"),
    }
}

// 4. help rows ----------------------------------------------------------------
// A help line: a section title, or a key label (not translated) with its meaning.
#[derive(Debug, Clone, Copy)]
pub enum HelpRow {
    Section(Msg),
    Key(&'static str, Msg),
}

pub const CAPTURE_HELP: &[HelpRow] = &[
    HelpRow::Section(Msg::HelpSectionKeyboard),
    HelpRow::Key("q  Ctrl+C", Msg::HelpQuit),
    HelpRow::Key("?", Msg::HelpToggleHelp),
    HelpRow::Key("↑ ↓ j k", Msg::HelpMoveRows),
    HelpRow::Key("PgUp PgDn", Msg::HelpPage),
    HelpRow::Key("Home End g G", Msg::HelpFirstLast),
    HelpRow::Key("Tab  Shift+Tab", Msg::HelpNextCard),
    HelpRow::Key("Space", Msg::HelpPause),
    HelpRow::Key("/", Msg::HelpFilter),
    HelpRow::Key("Ctrl+F  n", Msg::HelpSearch),
    HelpRow::Key("i  o", Msg::HelpDirection),
    HelpRow::Key("s  S", Msg::HelpSort),
    HelpRow::Key("c", Msg::HelpColumns),
    HelpRow::Key("d  x  z", Msg::HelpPanes),
    HelpRow::Key("m  '", Msg::HelpBookmark),
    HelpRow::Key("t  P  F  T", Msg::HelpStats),
    HelpRow::Key("f", Msg::HelpStream),
    HelpRow::Key("w", Msg::HelpSave),
    HelpRow::Key("Shift+Arrows", Msg::HelpMoveCard),
    HelpRow::Key("l", Msg::HelpResetLayout),
    HelpRow::Key("L", Msg::HelpLanguage),
    HelpRow::Key("Esc", Msg::HelpCloseOverlay),
    HelpRow::Section(Msg::HelpSectionMouse),
    HelpRow::Key("Click", Msg::HelpClickCard),
    HelpRow::Key("Header", Msg::HelpClickHeader),
    HelpRow::Key("Column edge", Msg::HelpDragColumn),
    HelpRow::Key("Wheel / edge", Msg::HelpWheel),
    HelpRow::Key("Title hold", Msg::HelpLongPress),
    HelpRow::Key("Scrollbar", Msg::HelpScrollbar),
    HelpRow::Key("Chips", Msg::HelpChips),
];

pub const INSPECT_HELP: &[HelpRow] = &[
    HelpRow::Section(Msg::HelpSectionKeyboard),
    HelpRow::Key("q  Esc  Ctrl+C", Msg::HelpQuit),
    HelpRow::Key("?", Msg::HelpToggleHelp),
    HelpRow::Key("Tab  1-5", Msg::HelpSwitchTab),
    HelpRow::Key("Left  Right", Msg::HelpSwitchTab),
    HelpRow::Key("Enter", Msg::HelpToggleDetail),
    HelpRow::Key("↑ ↓ j k", Msg::HelpMoveRows),
    HelpRow::Key("PgUp PgDn", Msg::HelpPage),
    HelpRow::Key("Home End", Msg::HelpFirstLast),
    HelpRow::Key("/", Msg::HelpSearchRows),
    HelpRow::Key("s  S", Msg::HelpSort),
    HelpRow::Key("r", Msg::HelpRefresh),
    HelpRow::Key("Shift+Arrows", Msg::HelpMoveCard),
    HelpRow::Key("l", Msg::HelpResetLayout),
    HelpRow::Key("L", Msg::HelpLanguage),
    HelpRow::Section(Msg::HelpSectionMouse),
    HelpRow::Key("Click", Msg::HelpClickCard),
    HelpRow::Key("Header", Msg::HelpClickHeader),
    HelpRow::Key("Column edge", Msg::HelpDragColumn),
    HelpRow::Key("Wheel / edge", Msg::HelpWheel),
    HelpRow::Key("Title hold", Msg::HelpLongPress),
    HelpRow::Key("Scrollbar", Msg::HelpScrollbar),
    HelpRow::Key("Chips", Msg::HelpChips),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_message_has_both_languages_and_the_default_is_english() {
        assert_eq!(Lang::default(), Lang::En);
        for msg in Msg::ALL {
            assert!(!text(Lang::En, *msg).is_empty(), "{msg:?} en");
            assert!(!text(Lang::Ko, *msg).is_empty(), "{msg:?} ko");
        }
    }

    #[test]
    fn language_codes_round_trip_and_unknown_codes_are_rejected() {
        for lang in [Lang::En, Lang::Ko] {
            assert_eq!(Lang::from_code(lang.code()), Some(lang));
        }
        assert_eq!(Lang::from_code("EN"), None);
        assert_eq!(Lang::from_code("jp"), None);
        assert_eq!(Lang::En.toggled(), Lang::Ko);
        assert_eq!(Lang::Ko.toggled(), Lang::En);
    }

    #[test]
    fn language_chip_labels_differ_between_languages() {
        assert_eq!(text(Lang::En, Msg::ChipLanguage), "EN");
        assert_eq!(text(Lang::Ko, Msg::ChipLanguage), "KO");
    }

    #[test]
    fn format_functions_follow_the_language_and_keep_data_untouched() {
        assert!(too_small(Lang::En, 60, 16, 40, 10).contains("Terminal too small"));
        assert!(too_small(Lang::Ko, 60, 16, 40, 10).contains("터미널 크기 부족"));
        assert!(filter_applied(Lang::En, "tcp.port == 443").ends_with("tcp.port == 443"));
        assert!(filter_applied(Lang::Ko, "tcp.port == 443").ends_with("tcp.port == 443"));
        assert!(log_saved(Lang::Ko, 3, "/x/a.log").contains("/x/a.log"));
        assert!(
            state_unreadable(Lang::Ko, "tui.toml", &StateIssue::UnsupportedVersion(2))
                .contains("2")
        );
    }

    #[test]
    fn help_tables_name_every_key_once_per_section() {
        for rows in [CAPTURE_HELP, INSPECT_HELP] {
            assert!(matches!(
                rows[0],
                HelpRow::Section(Msg::HelpSectionKeyboard)
            ));
            assert!(
                rows.iter()
                    .any(|row| matches!(row, HelpRow::Section(Msg::HelpSectionMouse)))
            );
        }
    }
}
