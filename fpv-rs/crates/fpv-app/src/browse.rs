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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Browse {
    pub view: View,
    pub session: Option<String>,
    pub filter_session: Option<String>,
    pub liked: bool,
    pub labels: Vec<String>,
    pub all_labels: bool,
    pub figure_labels: Vec<String>,
    pub sort: Sort,
    pub page: usize,
    pub page_size: usize,
}

impl Default for Browse {
    fn default() -> Self {
        Self {
            view: View::Sessions,
            session: None,
            filter_session: None,
            liked: false,
            labels: vec![],
            all_labels: false,
            figure_labels: vec![],
            sort: Sort::Newest,
            page: 0,
            page_size: 24,
        }
    }
}

pub fn same(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

impl Browse {
    pub fn matches(&self, session: &SessionView, flight: &FlightView) -> bool {
        let wanted = if self.view == View::Sessions {
            &self.session
        } else {
            &self.filter_session
        };
        if wanted.as_ref().is_some_and(|id| id != &session.id) || (self.liked && !flight.liked) {
            return false;
        }
        if self.view == View::Figures || self.labels.is_empty() {
            return true;
        }
        let has = |label: &String| {
            flight
                .labels
                .iter()
                .chain(&session.labels)
                .any(|tag| same(tag, label))
        };
        if self.all_labels {
            self.labels.iter().all(has)
        } else {
            self.labels.iter().any(has)
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
        items.sort_by(|(a, af), (b, bf)| match self.sort {
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

/// What a re-segment remakes, and so loses: the session's DVR flights with
/// their titles, likes and tags. Action-cam takes are not touched.
pub fn resegment_losses(session: &SessionView) -> Vec<String> {
    let flights: Vec<&FlightView> = session.flights.iter().filter(|f| !f.action_cam).collect();
    if flights.is_empty() {
        // A first segmentation: there is nothing to lose.
        return vec![];
    }
    let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    let count = |keep: fn(&FlightView) -> bool| flights.iter().filter(|f| keep(f)).count();
    let mut lost = vec![format!(
        "The {} of this session will be found again from scratch and replaced. Lost with them:",
        plural(flights.len(), "DVR flight")
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
                // A take is not remade: its title is safe.
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
                "The 3 DVR flights of this session will be found again from scratch and replaced. Lost with them:",
                "1 title",
                "2 likes",
                "the tags of 1 flight",
                "every boundary corrected by hand",
            ]
        );
        session.flights.retain(|f| f.action_cam);
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
        browse.labels = vec!["race".into(), "FREESTYLE".into()];
        browse.all_labels = true;
        assert_eq!(
            browse
                .flights(&sessions)
                .iter()
                .map(|(_, f)| f.id.as_str())
                .collect::<Vec<_>>(),
            ["one"]
        );
        browse.all_labels = false;
        assert_eq!(browse.flights(&sessions).len(), 2);
        browse.liked = true;
        assert_eq!(browse.flights(&sessions).len(), 1);
        browse.filter_session = Some("alps".into());
        assert!(browse.flights(&sessions).is_empty());
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
            sort: Sort::Oldest,
            ..Browse::default()
        };
        let ids = |browse: &Browse| {
            browse
                .flights(&sessions)
                .iter()
                .map(|(_, f)| f.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&browse), ["old", "one", "two", "long"]);
        browse.sort = Sort::Newest;
        assert_eq!(ids(&browse), ["long", "two", "one", "old"]);
        browse.sort = Sort::Longest;
        assert_eq!(ids(&browse), ["long", "one", "two", "old"]);
    }
}
