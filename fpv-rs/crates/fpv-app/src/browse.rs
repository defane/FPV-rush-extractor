//! Browser rules, independent of window rendering.
use fpv_library::{FlightView, SessionView};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum View {
    #[default]
    Sessions,
    Flights,
    Figures,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sort {
    #[default]
    Newest,
    Oldest,
    Longest,
}

/// One view's filters and sort. Each view keeps its own, as the Tauri app
/// did: a tag picked in All flights must not hide flights inside a session.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Filters {
    /// One session's flights only (All flights, Figures).
    pub session: Option<String>,
    pub liked: bool,
    /// Flight and session tags (All flights), or figure names (Figures).
    pub labels: Vec<String>,
    /// Every tag rather than any (All flights).
    pub all_labels: bool,
    pub sort: Sort,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Browse {
    pub view: View,
    /// The session open in the Sessions view.
    pub session: Option<String>,
    /// A session's flights in their flying order, oldest first.
    pub in_session: Filters,
    pub flights: Filters,
    pub figures: Filters,
    pub page: usize,
    pub page_size: usize,
    /// The sidebar's page of sessions.
    pub session_page: usize,
}

/// Sessions per page in the sidebar.
pub const SESSIONS_PER_PAGE: usize = 10;

/// The sidebar's page follows the session opened from elsewhere — a figure
/// card, All flights — but only when that session changes: a library
/// refresh, which every ending task causes, must not pull the viewer back
/// from the page they turned to.
pub fn follow(
    ids: &[&str],
    opened: Option<&str>,
    followed: &mut Option<String>,
    page: &mut usize,
    per_page: usize,
) {
    if opened == followed.as_deref() {
        return;
    }
    *followed = opened.map(str::to_owned);
    if let Some(index) = opened.and_then(|id| ids.iter().position(|known| *known == id)) {
        *page = index / per_page.max(1);
    }
}

impl Default for Browse {
    fn default() -> Self {
        Self {
            view: View::Sessions,
            session: None,
            in_session: Filters {
                sort: Sort::Oldest,
                ..Filters::default()
            },
            flights: Filters::default(),
            figures: Filters::default(),
            page: 0,
            page_size: 24,
            session_page: 0,
        }
    }
}

pub fn same(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

impl Browse {
    /// The filters of the view on screen.
    pub fn filters(&self) -> &Filters {
        match self.view {
            View::Sessions => &self.in_session,
            View::Flights => &self.flights,
            View::Figures => &self.figures,
        }
    }

    pub fn filters_mut(&mut self) -> &mut Filters {
        match self.view {
            View::Sessions => &mut self.in_session,
            View::Flights => &mut self.flights,
            View::Figures => &mut self.figures,
        }
    }

    pub fn matches(&self, session: &SessionView, flight: &FlightView) -> bool {
        let filters = self.filters();
        let wanted = if self.view == View::Sessions {
            &self.session
        } else {
            &filters.session
        };
        if wanted.as_ref().is_some_and(|id| id != &session.id) || (filters.liked && !flight.liked) {
            return false;
        }
        // Tags filter All flights only; figure names are matched per figure.
        if self.view != View::Flights || filters.labels.is_empty() {
            return true;
        }
        let has = |label: &String| {
            flight
                .labels
                .iter()
                .chain(&session.labels)
                .any(|tag| same(tag, label))
        };
        if filters.all_labels {
            filters.labels.iter().all(has)
        } else {
            filters.labels.iter().any(has)
        }
    }

    pub fn flights<'a>(
        &self,
        sessions: &'a [SessionView],
    ) -> Vec<(&'a SessionView, &'a FlightView)> {
        let mut items: Vec<_> = sessions
            .iter()
            .flat_map(|session| session.flights.iter().map(move |flight| (session, flight)))
            .filter(|(s, f)| self.matches(s, f))
            .collect();
        items.sort_by(|(a, af), (b, bf)| match self.filters().sort {
            Sort::Longest => bf.duration.total_cmp(&af.duration),
            sort => {
                let order = a
                    .date
                    .cmp(&b.date)
                    .then_with(|| a.id.cmp(&b.id))
                    .then_with(|| af.order.cmp(&bf.order));
                if sort == Sort::Newest {
                    order.reverse()
                } else {
                    order
                }
            }
        });
        items
    }
}

pub fn page_range(total: usize, page: &mut usize, size: usize) -> std::ops::Range<usize> {
    let size = size.max(1);
    *page = (*page).min(total.saturating_sub(1) / size);
    let start = page.saturating_mul(size);
    start..start.saturating_add(size).min(total)
}

/// Page indexes to show, with `None` representing an ellipsis.
pub fn visible_pages(current: usize, total: usize) -> Vec<Option<usize>> {
    if total <= 7 {
        return (0..total).map(Some).collect();
    }
    let current = current.min(total - 1);
    let start = current.saturating_sub(2).max(1);
    let end = current.saturating_add(2).min(total - 2);
    let mut pages = vec![Some(0)];
    if start > 1 {
        pages.push(None);
    }
    pages.extend((start..=end).map(Some));
    if end < total - 2 {
        pages.push(None);
    }
    pages.push(Some(total - 1));
    pages
}

pub fn clock(seconds: f64) -> String {
    let seconds = if seconds.is_finite() {
        seconds.max(0.0)
    } else {
        0.0
    };
    format!("{}:{:04.1}", (seconds / 60.0) as u64, seconds % 60.0)
}

pub fn flight_name(flight: &FlightView) -> String {
    flight
        .title
        .clone()
        .unwrap_or_else(|| format!("Flight {}", flight.order))
}

pub fn labels(text: &str) -> Vec<String> {
    let mut result: Vec<String> = vec![];
    for label in text.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if !result.iter().any(|known| same(known, label)) {
            result.push(label.into());
        }
    }
    result
}

/// The names a filter offers, each with what picking it would list: in All
/// flights, how many flights carry the tag (theirs or their session's, each
/// flight once); in Figures, how many figures bear the name. Sorted by name,
/// in the spelling first met.
pub fn label_counts(sessions: &[SessionView], view: View) -> Vec<(String, usize)> {
    let mut counts: Vec<(String, usize)> = vec![];
    let mut count = |name: &str| match counts.iter_mut().find(|(known, _)| same(known, name)) {
        Some((_, n)) => *n += 1,
        None => counts.push((name.to_owned(), 1)),
    };
    for session in sessions {
        for flight in &session.flights {
            if view == View::Figures {
                flight
                    .figures
                    .iter()
                    .for_each(|figure| count(&figure.label));
            } else {
                card_labels(flight, session)
                    .iter()
                    .for_each(|(label, _)| count(label));
            }
        }
    }
    counts.sort_by_key(|(name, _)| name.to_lowercase());
    counts
}

/// How many flights — or figures, in Figures — the filters let through.
pub fn match_count(browse: &Browse, sessions: &[SessionView]) -> usize {
    let flights = browse.flights(sessions);
    if browse.view != View::Figures {
        return flights.len();
    }
    let names = &browse.figures.labels;
    flights
        .iter()
        .flat_map(|(_, flight)| &flight.figures)
        .filter(|figure| names.is_empty() || names.iter().any(|name| same(name, &figure.label)))
        .count()
}

/// A session row's second line: "2026-10-23 · 1 file · 2 flights", the day
/// left out when there is none.
pub fn session_meta(day: &str, files: usize, flights: usize) -> String {
    let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    let counts = format!("{} · {}", plural(files, "file"), plural(flights, "flight"));
    if day.is_empty() {
        counts
    } else {
        format!("{day} · {counts}")
    }
}

/// The session header's button: "Re-segment" once flights exist — a
/// re-segment remakes them — "Segment" before.
pub fn segment_label(session: &SessionView) -> &'static str {
    if session.flights.iter().any(|f| !f.reduced) {
        "Re-segment"
    } else {
        "Segment"
    }
}

/// What an empty grid offers to do about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmptyAction {
    Segment,
    ResetFilters,
    Nothing,
}

/// What an empty grid says: why there is nothing, and what to do — the
/// Tauri app's empty states. `searching` is text in the search field.
pub fn empty_state(
    view: View,
    session_has_flights: bool,
    filters: &Filters,
    searching: bool,
) -> (&'static str, &'static str, EmptyAction) {
    let filtered = searching
        || filters.liked
        || (view != View::Sessions && (filters.session.is_some() || !filters.labels.is_empty()));
    match view {
        View::Sessions if !session_has_flights => (
            "No flights yet",
            "Segment the recordings to find the flights in them.",
            EmptyAction::Segment,
        ),
        View::Sessions if filters.liked && !searching => (
            "No liked flights",
            "A heart on a card likes its flight.",
            EmptyAction::ResetFilters,
        ),
        View::Figures if !filtered => (
            "No figures here",
            "Tag figures in the player — Mark In, name it, Enter.",
            EmptyAction::Nothing,
        ),
        View::Flights if !filtered => (
            "No flights yet",
            "Import recordings and segment them to see their flights here.",
            EmptyAction::Nothing,
        ),
        View::Figures => (
            "No figures match these filters",
            "",
            EmptyAction::ResetFilters,
        ),
        _ => (
            "No flights match these filters",
            "",
            EmptyAction::ResetFilters,
        ),
    }
}

/// A size as people read it: "548 KB", "7.5 MB", "1.2 GB".
pub fn size(bytes: u64) -> String {
    let bytes = bytes as f64;
    if bytes < 1e6 {
        format!("{:.0} KB", bytes / 1e3)
    } else if bytes < 1e9 {
        format!("{:.1} MB", bytes / 1e6)
    } else {
        format!("{:.1} GB", bytes / 1e9)
    }
}

/// What emptying part of the cache asks first: what goes, how much, and
/// that nothing is lost.
pub fn empty_question(what: &str, bytes: u64) -> String {
    format!(
        "Empty {what} ({})? Nothing is lost: they are made again when needed.",
        size(bytes)
    )
}

/// The figure button: a point at the playhead, or — Mark In set — the range
/// from the mark to it.
pub fn add_figure_label(mark: Option<f64>, now: f64) -> String {
    match mark {
        Some(_) => format!("Add Range to {}", clock(now)),
        None => "Add Point".into(),
    }
}

/// "2 chapters" for an action cam's take in chapters, "2 recordings" for a
/// DVR flight across the DVR's cuts; nothing for a flight in one file.
pub fn parts_label(flight: &FlightView) -> Option<String> {
    (flight.recordings > 1).then(|| {
        let what = if flight.action_cam {
            "chapters"
        } else {
            "recordings"
        };
        format!("{} {what}", flight.recordings)
    })
}

/// A card's tags: the flight's own, then the session's it does not have
/// itself, each once, marked as inherited so the card can draw it apart.
pub fn card_labels(flight: &FlightView, session: &SessionView) -> Vec<(String, bool)> {
    let mut labels: Vec<(String, bool)> = vec![];
    for (label, inherited) in flight
        .labels
        .iter()
        .map(|l| (l, false))
        .chain(session.labels.iter().map(|l| (l, true)))
    {
        if !labels.iter().any(|(known, _)| same(known, label)) {
            labels.push((label.clone(), inherited));
        }
    }
    labels
}

/// The hover strip's frame `at` flight seconds in: the strip spreads `count`
/// frames over the flight, each the middle of its share.
pub fn strip_index(at: f64, duration: f64, count: usize) -> usize {
    if count == 0 || duration.is_nan() || duration <= 0.0 || !at.is_finite() {
        return 0;
    }
    ((at / duration).clamp(0.0, 0.999_999) * count as f64) as usize
}

/// The frame to show while the one wanted is still being made: the nearest
/// one already loaded, the earlier on a tie.
pub fn nearest_known(wanted: usize, count: usize, known: impl Fn(usize) -> bool) -> Option<usize> {
    (0..count)
        .filter(|&i| known(i))
        .min_by_key(|&i| (i.abs_diff(wanted), i))
}

/// What recorded a session, for its row: "DVR" (with "· reduced" or "·
/// partly reduced" when its recordings were cut down to their flights), then
/// each camera once, in the order its files came.
#[derive(Debug, Default, PartialEq)]
pub struct Sources {
    pub dvr: Option<&'static str>,
    pub cameras: Vec<String>,
}

pub fn session_sources(session: &SessionView) -> Sources {
    let mut cameras: Vec<String> = vec![];
    for file in session.files.iter().filter(|f| f.action_cam) {
        if !cameras.iter().any(|c| same(c, &file.source)) {
            cameras.push(file.source.clone());
        }
    }
    let dvr: Vec<_> = session.files.iter().filter(|f| !f.action_cam).collect();
    let cut = dvr.iter().filter(|f| f.reduced).count();
    Sources {
        dvr: (!dvr.is_empty()).then_some(match cut {
            0 => "DVR",
            n if n == dvr.len() => "DVR · reduced",
            _ => "DVR · partly reduced",
        }),
        cameras,
    }
}

/// The earlier import folders a session's recordings still sit in, each once:
/// what "Move to Import Folder…" would bring in.
pub fn elsewhere(session: &SessionView) -> Vec<String> {
    let mut folders: Vec<String> = vec![];
    for folder in session.files.iter().filter_map(|f| f.elsewhere.as_ref()) {
        if !folders.contains(folder) {
            folders.push(folder.clone());
        }
    }
    folders
}

/// How much of a session only the cloud drive holds: "in the cloud" when all
/// of it, "partly in the cloud" when some, nothing when it is all here.
pub fn cloud_state(session: &SessionView) -> Option<&'static str> {
    let away = session.files.iter().filter(|file| file.cloud).count();
    match away {
        0 => None,
        n if n == session.files.len() => Some("in the cloud"),
        _ => Some("partly in the cloud"),
    }
}

/// Names the library already uses that match what is being typed, each once
/// in the spelling first met — the tags across flights and sessions, or the
/// figures — those starting with it first, never one already `taken`.
pub fn suggest<'a>(
    names: impl IntoIterator<Item = &'a str>,
    typed: &str,
    taken: &[String],
) -> Vec<String> {
    let typed = typed.trim().to_lowercase();
    if typed.is_empty() {
        return vec![];
    }
    let mut found: Vec<String> = vec![];
    for name in names {
        let lower = name.to_lowercase();
        if lower.contains(&typed)
            && lower != typed
            && !taken.iter().any(|t| same(t, name))
            && !found.iter().any(|f| same(f, name))
        {
            found.push(name.to_owned());
        }
    }
    found.sort_by_key(|name| {
        (
            !name.to_lowercase().starts_with(&typed),
            name.to_lowercase(),
        )
    });
    found.truncate(5);
    found
}

/// What re-segmentation replaces for every codec; reduced clips are preserved.
pub fn resegment_losses(session: &SessionView) -> Vec<String> {
    let flights: Vec<&FlightView> = session.flights.iter().filter(|f| !f.reduced).collect();
    if flights.is_empty() {
        // A first segmentation: there is nothing to lose.
        return vec![];
    }
    let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    let count = |keep: fn(&FlightView) -> bool| flights.iter().filter(|f| keep(f)).count();
    let mut lost = vec![format!(
        "The {} of this session will be found again from scratch and replaced. Lost with them:",
        plural(flights.len(), "flight")
    )];
    let titled = count(|f| f.title.is_some());
    if titled > 0 {
        lost.push(plural(titled, "title"));
    }
    let liked = count(|f| f.liked);
    if liked > 0 {
        lost.push(plural(liked, "like"));
    }
    let tagged = count(|f| !f.labels.is_empty());
    if tagged > 0 {
        lost.push(format!("the tags of {}", plural(tagged, "flight")));
    }
    lost.push("every boundary corrected by hand".into());
    lost
}

#[cfg(test)]
mod tests {

    fn recording(
        source: &str,
        action_cam: bool,
        reduced: bool,
        elsewhere: Option<&str>,
    ) -> fpv_library::FileView {
        fpv_library::FileView {
            id: "f".into(),
            name: "PICT0032.AVI".into(),
            width: 640,
            height: 480,
            fps: 29.887,
            duration: 60.0,
            codec: "mjpeg".into(),
            action_cam,
            source: source.into(),
            format: String::new(),
            reduced,
            elsewhere: elsewhere.map(Into::into),
            cloud: false,
            freeable: false,
        }
    }

    #[test]
    fn the_sidebar_follows_a_session_opened_elsewhere_only_when_it_changes() {
        let ids: Vec<String> = (0..25).map(|n| format!("s{n}")).collect();
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        let (mut followed, mut page) = (None, 0);
        follow(&ids, Some("s23"), &mut followed, &mut page, 10);
        assert_eq!(page, 2, "s23 is on the third page");
        // The viewer turns back to the first page; a refresh keeps it there.
        page = 0;
        follow(&ids, Some("s23"), &mut followed, &mut page, 10);
        assert_eq!(page, 0);
        follow(&ids, Some("s12"), &mut followed, &mut page, 10);
        assert_eq!(page, 1, "another session opened: followed");
        follow(&ids, None, &mut followed, &mut page, 10);
        assert_eq!(page, 1, "closing it moves nothing");
    }

    #[test]
    fn a_filter_counts_what_each_name_would_list_and_what_matches() {
        let tagged = |id: &str, labels: &[&str]| flight(id, 1, false, labels, 60.0);
        let mut one = tagged("one", &["Gates", "freestyle"]);
        one.figures.push(fpv_library::TagView {
            id: "t".into(),
            label: "power loop".into(),
            note: None,
            start: 1.0,
            end: 2.0,
            marker: false,
        });
        let sessions = vec![
            session(
                "race",
                None,
                vec![one, tagged("two", &["gates"])],
                &["gates"],
            ),
            session("alps", None, vec![tagged("three", &[])], &["Gates"]),
        ];
        assert_eq!(
            label_counts(&sessions, View::Flights),
            [("freestyle".to_owned(), 1), ("Gates".to_owned(), 3)],
            "each flight once, its session's tag included"
        );
        assert_eq!(
            label_counts(&sessions, View::Figures),
            [("power loop".to_owned(), 1)]
        );
        let mut browse = Browse {
            view: View::Flights,
            ..Browse::default()
        };
        browse.flights.labels = vec!["freestyle".into()];
        assert_eq!(match_count(&browse, &sessions), 1);
        browse.view = View::Figures;
        assert_eq!(match_count(&browse, &sessions), 1);
        browse.figures.labels = vec!["split-s".into()];
        assert_eq!(match_count(&browse, &sessions), 0);
    }

    #[test]
    fn a_session_row_counts_in_words_and_leaves_out_a_missing_day() {
        assert_eq!(
            session_meta("2026-10-23", 1, 2),
            "2026-10-23 · 1 file · 2 flights"
        );
        assert_eq!(session_meta("", 3, 1), "3 files · 1 flight");
    }

    #[test]
    fn the_button_says_re_segment_for_every_existing_codec() {
        let take = FlightView {
            action_cam: true,
            ..flight("take", 1, false, &[], 60.0)
        };
        let mut letz = session("letz", None, vec![take], &[]);
        assert_eq!(
            segment_label(&letz),
            "Re-segment",
            "all codecs are analyzed"
        );
        letz.flights.push(flight("dvr", 2, false, &[], 60.0));
        assert_eq!(segment_label(&letz), "Re-segment");
    }

    #[test]
    fn an_empty_grid_says_why_and_what_to_do() {
        let none = Filters::default();
        let liked = Filters {
            liked: true,
            ..Filters::default()
        };
        let tagged = Filters {
            labels: vec!["gates".into()],
            ..Filters::default()
        };
        assert_eq!(
            empty_state(View::Sessions, false, &none, false).2,
            EmptyAction::Segment
        );
        assert_eq!(
            empty_state(View::Sessions, true, &liked, false).0,
            "No liked flights"
        );
        assert_eq!(
            empty_state(View::Sessions, true, &none, true),
            (
                "No flights match these filters",
                "",
                EmptyAction::ResetFilters
            )
        );
        assert_eq!(
            empty_state(View::Flights, true, &none, false).0,
            "No flights yet"
        );
        assert_eq!(
            empty_state(View::Flights, true, &tagged, false).2,
            EmptyAction::ResetFilters
        );
        assert_eq!(
            empty_state(View::Figures, true, &none, false).0,
            "No figures here"
        );
        assert_eq!(
            empty_state(View::Figures, true, &tagged, false).0,
            "No figures match these filters"
        );
    }

    #[test]
    fn sizes_read_as_people_say_them_and_emptying_asks_with_the_size() {
        assert_eq!(size(548_000), "548 KB");
        assert_eq!(size(7_500_000), "7.5 MB");
        assert_eq!(size(1_234_000_000), "1.2 GB");
        assert_eq!(
            empty_question("hover strips", 7_500_000),
            "Empty hover strips (7.5 MB)? Nothing is lost: they are made again when needed."
        );
    }

    #[test]
    fn the_figure_button_says_point_or_range() {
        assert_eq!(add_figure_label(None, 12.3), "Add Point");
        assert_eq!(add_figure_label(Some(4.7), 12.3), "Add Range to 0:12.3");
    }

    #[test]
    fn a_card_counts_chapters_or_recordings_and_marks_inherited_tags() {
        let take = FlightView {
            recordings: 2,
            action_cam: true,
            ..flight("take", 1, false, &["Gates"], 60.0)
        };
        assert_eq!(parts_label(&take).as_deref(), Some("2 chapters"));
        let dvr = FlightView {
            action_cam: false,
            ..take.clone()
        };
        assert_eq!(parts_label(&dvr).as_deref(), Some("2 recordings"));
        assert_eq!(
            parts_label(&FlightView {
                recordings: 1,
                ..dvr
            }),
            None
        );
        let race = session("race", None, vec![], &["gates", "Race"]);
        assert_eq!(
            card_labels(&take, &race),
            [("Gates".to_owned(), false), ("Race".to_owned(), true)]
        );
    }

    #[test]
    fn finds_a_moments_strip_frame_and_the_nearest_one_loaded() {
        assert_eq!(strip_index(0.0, 60.0, 24), 0);
        assert_eq!(strip_index(30.0, 60.0, 24), 12);
        assert_eq!(strip_index(60.0, 60.0, 24), 23, "the end is the last frame");
        assert_eq!(strip_index(f64::NAN, 60.0, 24), 0);
        assert_eq!(strip_index(10.0, 0.0, 24), 0);
        let loaded = [2usize, 9, 15];
        assert_eq!(nearest_known(10, 24, |i| loaded.contains(&i)), Some(9));
        assert_eq!(
            nearest_known(12, 24, |i| loaded.contains(&i)),
            Some(9),
            "a tie: the earlier"
        );
        assert_eq!(nearest_known(5, 24, |_| false), None);
    }

    #[test]
    fn names_what_recorded_a_session_and_where_its_recordings_were_left() {
        let mut letz = session("letz", None, vec![], &[]);
        assert_eq!(session_sources(&letz), Sources::default());
        letz.files = vec![
            recording("DVR", false, true, None),
            recording("GoPro HERO11", true, false, Some("/Volumes/Old")),
            recording("DVR", false, false, Some("/Volumes/Old")),
            recording("gopro hero11", true, false, None),
            recording("DJI", true, false, Some("/media/Older")),
        ];
        assert_eq!(
            session_sources(&letz),
            Sources {
                dvr: Some("DVR · partly reduced"),
                cameras: vec!["GoPro HERO11".into(), "DJI".into()],
            }
        );
        assert_eq!(elsewhere(&letz), ["/Volumes/Old", "/media/Older"]);
        letz.files.retain(|f| !f.action_cam && f.reduced);
        assert_eq!(session_sources(&letz).dvr, Some("DVR · reduced"));
        letz.files[0].reduced = false;
        assert_eq!(session_sources(&letz).dvr, Some("DVR"));
    }

    #[test]
    fn says_whether_a_session_is_all_or_partly_in_the_cloud() {
        let file = |cloud| fpv_library::FileView {
            id: "f".into(),
            name: "GH010042.MP4".into(),
            width: 1920,
            height: 1080,
            fps: 30.0,
            duration: 60.0,
            codec: "h264".into(),
            action_cam: true,
            source: "GoPro".into(),
            format: String::new(),
            reduced: false,
            elsewhere: None,
            cloud,
            freeable: !cloud,
        };
        let mut letz = session("letz", None, vec![], &[]);
        assert_eq!(cloud_state(&letz), None, "no recordings");
        letz.files = vec![file(false), file(false)];
        assert_eq!(cloud_state(&letz), None);
        letz.files = vec![file(true), file(false)];
        assert_eq!(cloud_state(&letz), Some("partly in the cloud"));
        letz.files = vec![file(true), file(true)];
        assert_eq!(cloud_state(&letz), Some("in the cloud"));
    }

    #[test]
    fn suggests_the_names_in_use_starting_with_what_is_typed_first() {
        let names = [
            "Freestyle",
            "gates",
            "Bando",
            "freestyle",
            "Power gates",
            "Gates",
            "Air gates",
        ];
        assert_eq!(
            suggest(names, "ga", &[]),
            ["gates", "Air gates", "Power gates"]
        );
        assert_eq!(suggest(names, "  FREE", &[]), ["Freestyle"]);
        // Already on the flight, or exactly what is typed: nothing to add.
        assert!(suggest(names, "ga", &["Gates".into()]).contains(&"Power gates".to_string()));
        assert!(!suggest(names, "ga", &["Gates".into()]).contains(&"gates".to_string()));
        assert!(suggest(names, "bando", &[]).is_empty());
        assert!(suggest(names, " ", &[]).is_empty());
        let many: Vec<String> = (0..9).map(|n| format!("split {n}")).collect();
        assert_eq!(
            suggest(many.iter().map(String::as_str), "split", &[]).len(),
            5
        );
    }

    #[test]
    fn counts_what_a_re_segment_loses_and_asks_nothing_the_first_time() {
        let dvr = |title: Option<&str>, liked, labels: &[&str]| FlightView {
            title: title.map(Into::into),
            ..flight("dvr", 1, liked, labels, 60.0)
        };
        let mut session = session(
            "letz",
            None,
            vec![
                dvr(Some("Bando"), true, &["gates"]),
                dvr(None, true, &[]),
                dvr(None, false, &[]),
                // H.264 takes are analyzed too, so their edits are counted.
                FlightView {
                    title: Some("Osmo".into()),
                    action_cam: true,
                    ..flight("osmo", 4, false, &[], 60.0)
                },
            ],
            &[],
        );
        assert_eq!(
            resegment_losses(&session),
            [
                "The 4 flights of this session will be found again from scratch and replaced. Lost with them:",
                "2 titles",
                "2 likes",
                "the tags of 1 flight",
                "every boundary corrected by hand",
            ]
        );
        session.flights.clear();
        assert!(
            resegment_losses(&session).is_empty(),
            "nothing segmented yet"
        );
    }

    use super::*;
    #[test]
    fn empty_and_deleted_pages_clamp_without_overflow() {
        let mut page = usize::MAX;
        assert_eq!(page_range(0, &mut page, 0), 0..0);
        page = 5;
        assert_eq!(page_range(25, &mut page, 24), 24..25);
        assert_eq!(page, 1);
    }
    #[test]
    fn visible_pages_include_current_edges_and_ellipses() {
        assert_eq!(
            visible_pages(0, 5),
            [Some(0), Some(1), Some(2), Some(3), Some(4)]
        );
        assert_eq!(
            visible_pages(5, 20),
            [
                Some(0),
                None,
                Some(3),
                Some(4),
                Some(5),
                Some(6),
                Some(7),
                None,
                Some(19)
            ]
        );
        assert_eq!(
            visible_pages(18, 20),
            [Some(0), None, Some(16), Some(17), Some(18), Some(19)]
        );
    }
    #[test]
    fn entered_labels_preserve_spelling_and_ignore_case_duplicates() {
        assert_eq!(
            labels(" Gates,freestyle, gates,, "),
            vec!["Gates", "freestyle"]
        );
    }
    #[test]
    fn invalid_clock_input_is_safe() {
        assert_eq!(clock(f64::NAN), "0:00.0");
        assert_eq!(clock(-1.0), "0:00.0");
    }

    fn flight(id: &str, order: i64, liked: bool, labels: &[&str], duration: f64) -> FlightView {
        FlightView {
            id: id.into(),
            order,
            title: None,
            liked,
            duration,
            clips: vec![],
            recordings: 1,
            cloud: false,
            freeable: false,
            action_cam: false,
            source: "DVR".into(),
            format: String::new(),
            reduced: false,
            cache_key: id.into(),
            labels: labels.iter().map(|s| (*s).into()).collect(),
            figures: vec![],
        }
    }

    fn session(
        id: &str,
        date: Option<&str>,
        flights: Vec<FlightView>,
        labels: &[&str],
    ) -> SessionView {
        SessionView {
            id: id.into(),
            name: id.into(),
            date: date.map(str::to_owned),
            files: vec![],
            flights,
            segmentable: true,
            reducible: false,
            detection_profile: "standard".into(),
            location: None,
            notes: None,
            labels: labels.iter().map(|s| (*s).into()).collect(),
        }
    }

    #[test]
    fn filters_combine_session_and_flight_tags_without_case() {
        let sessions = vec![
            session(
                "race",
                None,
                vec![
                    flight("one", 1, true, &["freestyle", "Gates"], 60.0),
                    flight("two", 2, false, &["gates"], 20.0),
                ],
                &["Race"],
            ),
            session(
                "alps",
                None,
                vec![flight("three", 1, true, &["cinematic"], 360.0)],
                &[],
            ),
        ];
        let mut browse = Browse {
            view: View::Flights,
            ..Browse::default()
        };
        assert_eq!(browse.flights(&sessions).len(), 3);
        browse.flights.labels = vec!["race".into(), "FREESTYLE".into()];
        browse.flights.all_labels = true;
        assert_eq!(
            browse
                .flights(&sessions)
                .iter()
                .map(|(_, f)| f.id.as_str())
                .collect::<Vec<_>>(),
            ["one"]
        );
        browse.flights.all_labels = false;
        assert_eq!(browse.flights(&sessions).len(), 2);
        browse.flights.liked = true;
        assert_eq!(browse.flights(&sessions).len(), 1);
        browse.flights.session = Some("alps".into());
        assert!(browse.flights(&sessions).is_empty());
    }

    #[test]
    fn each_view_keeps_its_own_filters_and_sort() {
        let sessions = vec![session(
            "race",
            None,
            vec![
                flight("one", 1, true, &["gates"], 60.0),
                flight("two", 2, false, &[], 20.0),
            ],
            &[],
        )];
        let mut browse = Browse::default();
        assert_eq!(
            browse.in_session.sort,
            Sort::Oldest,
            "a session reads in flying order"
        );
        assert_eq!(
            (browse.flights.sort, browse.figures.sort),
            (Sort::Newest, Sort::Newest)
        );
        // Filters set in All flights…
        browse.view = View::Flights;
        browse.filters_mut().labels = vec!["gates".into()];
        browse.filters_mut().liked = true;
        browse.filters_mut().sort = Sort::Longest;
        assert_eq!(browse.flights(&sessions).len(), 1);
        // …hide nothing in the session, which keeps its own order.
        browse.view = View::Sessions;
        browse.session = Some("race".into());
        let ids: Vec<_> = browse
            .flights(&sessions)
            .iter()
            .map(|(_, f)| f.id.clone())
            .collect();
        assert_eq!(ids, ["one", "two"]);
        // Back in All flights, they are still set.
        browse.view = View::Flights;
        assert_eq!(browse.filters().sort, Sort::Longest);
        assert_eq!(browse.flights(&sessions).len(), 1);
        // Figure names pick figures, not flights by their tags.
        browse.view = View::Figures;
        browse.filters_mut().labels = vec!["power loop".into()];
        assert_eq!(browse.flights(&sessions).len(), 2);
    }

    #[test]
    fn sorts_undated_sessions_then_dates_then_flight_order() {
        let sessions = vec![
            session(
                "alps",
                Some("2026-10-17"),
                vec![flight("long", 1, false, &[], 360.0)],
                &[],
            ),
            session(
                "race",
                Some("2026-08-23"),
                vec![
                    flight("one", 1, false, &[], 60.0),
                    flight("two", 2, false, &[], 20.0),
                ],
                &[],
            ),
            session(
                "unknown",
                None,
                vec![flight("old", 1, false, &[], 10.0)],
                &[],
            ),
        ];
        let mut browse = Browse {
            view: View::Flights,
            ..Browse::default()
        };
        browse.flights.sort = Sort::Oldest;
        let ids = |browse: &Browse| {
            browse
                .flights(&sessions)
                .iter()
                .map(|(_, f)| f.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&browse), ["old", "one", "two", "long"]);
        browse.flights.sort = Sort::Newest;
        assert_eq!(ids(&browse), ["long", "two", "one", "old"]);
        browse.flights.sort = Sort::Longest;
        assert_eq!(ids(&browse), ["long", "one", "two", "old"]);
    }
}
