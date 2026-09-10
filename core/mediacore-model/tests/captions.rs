//! Captions: the model, and the SRT/WebVTT round trip.
//!
//! Heavy on **malformed input** on purpose. Caption files come from a dozen
//! tools and are hand-edited by people, so a parser that only accepts the
//! specification is a parser that fails on real transcripts. The rule
//! throughout: recover what can be recovered, drop what cannot, never fail the
//! whole file over one bad cue.

use mediacore_model::captions::*;
use mediacore_model::time::Ticks;

fn secs(s: f64) -> Ticks { Ticks::from_seconds(s) }

fn caps(v: Vec<Caption>) -> Captions {
    let mut c = Captions::default();
    c.set(v);
    c
}

// ---------------------------------------------------------------- the model

#[test]
fn captions_are_kept_in_time_order_however_they_arrive() {
    // Lookup is a binary search; an out-of-order list shows the wrong line.
    let c = caps(vec![
        Caption::new(secs(4.0), secs(5.0), "third"),
        Caption::new(secs(0.0), secs(1.0), "first"),
        Caption::new(secs(2.0), secs(3.0), "second"),
    ]);
    let order: Vec<&str> = c.as_slice().iter().map(|c| c.text.as_str()).collect();
    assert_eq!(order, ["first", "second", "third"]);
}

#[test]
fn empty_and_backwards_captions_are_dropped() {
    // A zero-length caption can never be shown, and a blank one would flash an
    // empty box over the video.
    let c = caps(vec![
        Caption::new(secs(1.0), secs(1.0), "zero length"),
        Caption::new(secs(3.0), secs(2.0), "backwards"),
        Caption::new(secs(4.0), secs(5.0), "   "),
        Caption::new(secs(6.0), secs(7.0), "kept"),
    ]);
    assert_eq!(c.len(), 1);
    assert_eq!(c.as_slice()[0].text, "kept");
}

#[test]
fn the_right_caption_is_found_at_an_instant() {
    let c = caps(vec![
        Caption::new(secs(0.0), secs(2.0), "one"),
        Caption::new(secs(2.0), secs(4.0), "two"),
    ]);
    assert_eq!(c.at(secs(0.0)).map(|c| c.text.as_str()), Some("one"));
    assert_eq!(c.at(secs(1.9)).map(|c| c.text.as_str()), Some("one"));
    // The boundary belongs to the NEXT caption: end is exclusive, so two
    // adjacent captions never both claim the same instant.
    assert_eq!(c.at(secs(2.0)).map(|c| c.text.as_str()), Some("two"));
    assert_eq!(c.at(secs(9.0)), None);
}

#[test]
fn when_captions_overlap_the_most_recent_one_wins() {
    // Speakers interrupt each other and transcripts overlap. Showing the older
    // line would leave the viewer reading something already finished.
    let c = caps(vec![
        Caption::new(secs(0.0), secs(5.0), "long one"),
        Caption::new(secs(2.0), secs(3.0), "interruption"),
    ]);
    assert_eq!(c.at(secs(1.0)).map(|c| c.text.as_str()), Some("long one"));
    assert_eq!(c.at(secs(2.5)).map(|c| c.text.as_str()), Some("interruption"));
    assert_eq!(c.at(secs(4.0)).map(|c| c.text.as_str()), Some("long one"));
}

#[test]
fn shifting_moves_everything_and_never_goes_negative() {
    // The usual complaint about a transcript is that it is consistently early
    // or late. Shifting past zero must clamp rather than produce a caption at
    // a negative time, which nothing downstream expects.
    let mut c = caps(vec![
        Caption::new(secs(1.0), secs(2.0), "a"),
        Caption::new(secs(3.0), secs(4.0), "b"),
    ]);
    c.shift(secs(10.0));
    assert_eq!(c.as_slice()[0].start, secs(11.0));
    c.shift(secs(-100.0));
    assert!(c.as_slice().iter().all(|c| c.start.0 >= 0 && c.end.0 >= 0));
}

// ---------------------------------------------------------------- SRT

#[test]
fn srt_round_trips() {
    let c = caps(vec![
        Caption::new(secs(0.5), secs(2.25), "Hello there."),
        Caption::new(secs(3.0), secs(4.5), "Second line\nover two rows."),
    ]);
    let srt = c.to_srt();
    assert!(srt.starts_with("1\n00:00:00,500 --> 00:00:02,250\n"), "got:\n{srt}");
    let back = Captions::parse_subtitles(&srt);
    assert_eq!(back.len(), 2);
    assert_eq!(back[0].text, "Hello there.");
    assert_eq!(back[1].text, "Second line\nover two rows.");
    assert_eq!(back[0].start, secs(0.5));
    assert_eq!(back[1].end, secs(4.5));
}

#[test]
fn vtt_round_trips_and_carries_its_header() {
    let c = caps(vec![Caption::new(secs(1.0), secs(2.0), "Hi")]);
    let vtt = c.to_vtt();
    assert!(vtt.starts_with("WEBVTT"), "a .vtt without its header is rejected by players");
    assert!(vtt.contains("00:00:01.000 --> 00:00:02.000"), "got:\n{vtt}");
    let back = Captions::parse_subtitles(&vtt);
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].text, "Hi");
}

#[test]
fn an_hour_in_is_still_right() {
    // Off-by-one in the hour field is the classic subtitle bug, and it only
    // shows up on long recordings.
    let c = caps(vec![Caption::new(secs(3661.5), secs(3662.0), "late")]);
    assert!(c.to_srt().contains("01:01:01,500 --> 01:01:02,000"), "{}", c.to_srt());
    let back = Captions::parse_subtitles(&c.to_srt());
    assert_eq!(back[0].start, secs(3661.5));
}

// ---------------------------------------------------------------- real files

#[test]
fn a_file_with_no_sequence_numbers_still_parses() {
    // Perfectly common, and not actually valid SRT.
    let text = "00:00:01,000 --> 00:00:02,000\nOne\n\n00:00:03,000 --> 00:00:04,000\nTwo\n";
    let c = Captions::parse_subtitles(text);
    assert_eq!(c.len(), 2);
    assert_eq!(c[1].text, "Two");
}

#[test]
fn missing_blank_lines_between_cues_are_tolerated() {
    // A new timing line ends the previous cue even when the separator is gone.
    let text = "1\n00:00:01,000 --> 00:00:02,000\nOne\n2\n00:00:03,000 --> 00:00:04,000\nTwo\n";
    let c = Captions::parse_subtitles(text);
    assert_eq!(c.len(), 2, "got {c:?}");
    assert_eq!(c[0].text, "One");
}

#[test]
fn webvtt_cue_settings_and_blocks_are_ignored() {
    let text = "WEBVTT\n\nNOTE this is a comment\n\nSTYLE\n::cue { color: red }\n\n\
                intro\n00:00:01.000 --> 00:00:02.000 align:start position:10%\nHello\n";
    let c = Captions::parse_subtitles(text);
    assert_eq!(c.len(), 1, "got {c:?}");
    assert_eq!(c[0].text, "Hello");
    assert_eq!(c[0].end, secs(2.0));
}

#[test]
fn a_byte_order_mark_does_not_eat_the_first_cue() {
    let text = "\u{feff}1\n00:00:01,000 --> 00:00:02,000\nFirst\n";
    assert_eq!(Captions::parse_subtitles(text).len(), 1);
}

#[test]
fn windows_line_endings_do_not_leave_stray_carriage_returns() {
    let text = "1\r\n00:00:01,000 --> 00:00:02,000\r\nHello\r\n\r\n";
    let c = Captions::parse_subtitles(text);
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].text, "Hello", "a trailing \\r would corrupt every line drawn");
}

#[test]
fn one_broken_cue_does_not_lose_the_rest_of_the_transcript() {
    // The governing rule: ninety-eight recovered lines beat a hundred rejected.
    let text = "1\n00:00:01,000 --> 00:00:02,000\nGood\n\n\
                2\nnot a timestamp at all\nOrphaned\n\n\
                3\n00:00:05,000 --> 00:00:06,000\nAlso good\n";
    let c = Captions::parse_subtitles(text);
    assert_eq!(c.len(), 2, "got {c:?}");
    assert_eq!(c[1].text, "Also good");
}

#[test]
fn timestamps_are_accepted_in_the_shapes_that_actually_occur() {
    let text = "00:01.500 --> 00:02.000\nNo hour field\n\n\
                00:00:03.5 --> 00:00:04.25\nShort fractions\n";
    let c = Captions::parse_subtitles(text);
    assert_eq!(c.len(), 2, "got {c:?}");
    assert_eq!(c[0].start, secs(1.5), "MM:SS.mmm should be accepted");
    // "3.5" is 500 ms, not 5 ms.
    assert_eq!(c[1].start, secs(3.5));
    assert_eq!(c[1].end, secs(4.25));
}

#[test]
fn nonsense_never_panics_and_never_invents_captions() {
    for text in ["", "\n\n\n", "WEBVTT\n", "hello world", "-->", "99:99:99,999 --> 0"] {
        let c = Captions::parse_subtitles(text);
        assert!(c.is_empty(), "{text:?} produced {c:?}");
    }
}

#[test]
fn a_transcript_survives_being_written_and_read_a_second_time() {
    // Idempotence: exporting, reimporting and exporting again must not drift.
    let c = caps(Captions::parse_subtitles(
        "1\n00:00:01,000 --> 00:00:02,500\nLine one\n\n2\n00:00:03,000 --> 00:00:04,000\nLine two\n"));
    let once = c.to_srt();
    let twice = caps(Captions::parse_subtitles(&once)).to_srt();
    assert_eq!(once, twice);
}

// ------------------------------------------------- captions in the document
//
// Captions live on the Timeline rather than the Project specifically so they
// go through the one edit funnel — and therefore get undo, redo, autosave
// journalling and crash recovery without a second code path. These assert that
// they actually do, because "it should work by construction" is how things
// quietly do not.

use mediacore_model::command::Edit;
use mediacore_model::project::{Project, Session};

fn sample() -> Captions {
    caps(vec![
        Caption::new(secs(1.0), secs(2.0), "First line"),
        Caption::new(secs(3.0), secs(4.0), "Second line"),
    ])
}

#[test]
fn setting_captions_is_one_undoable_edit() {
    // A transcript is hundreds of captions. Undo must take back the whole
    // transcription, not one caption at a time.
    let mut s = Session::new(Project::default());
    let before = s.project.timeline.captions.clone();
    assert!(before.is_empty());

    s.apply(Edit::SetCaptions { from: before.clone(), to: sample() }).unwrap();
    assert_eq!(s.project.timeline.captions.len(), 2);

    s.undo().unwrap();
    assert!(s.project.timeline.captions.is_empty(), "undo left captions behind");

    s.redo().unwrap();
    assert_eq!(s.project.timeline.captions.len(), 2, "redo did not restore them");
}

#[test]
fn captions_survive_a_save_and_reopen() {
    let mut s = Session::new(Project::default());
    let mut with_burn = sample();
    with_burn.burn_in = true;
    with_burn.preset = 1;
    s.apply(Edit::SetCaptions { from: Captions::default(), to: with_burn }).unwrap();

    let json = s.project.to_json().unwrap();
    let back = Project::from_json(&json).unwrap();
    assert_eq!(back.timeline.captions.len(), 2, "the transcript was lost on save");
    assert_eq!(back.timeline.captions.as_slice()[1].text, "Second line");
    assert!(back.timeline.captions.burn_in, "the burn-in setting was lost");
    assert_eq!(back.timeline.captions.preset, 1, "the caption preset was lost");
    assert_eq!(back, s.project);
}

#[test]
fn a_project_without_captions_does_not_mention_them() {
    let s = Session::new(Project::default());
    assert!(!s.project.to_json().unwrap().contains("captions"));
}

#[test]
fn the_burned_in_caption_reaches_the_render_plan() {
    // Burning in reuses the text pipeline: the caption becomes an ordinary text
    // layer. If it never reaches the plan, the export silently has no captions
    // and every test on the caption list still passes.
    use mediacore_model::render::plan_at;
    let mut s = Session::new(Project::default());
    let mut c = sample();
    c.burn_in = true;
    s.apply(Edit::SetCaptions { from: Captions::default(), to: c }).unwrap();

    let during = plan_at(&s.project.timeline, secs(1.5));
    assert!(during.video.iter().any(|l| l.is_caption && l.is_text),
            "no caption layer at 1.5s");

    let between = plan_at(&s.project.timeline, secs(2.5));
    assert!(!between.video.iter().any(|l| l.is_caption),
            "a caption layer appeared where no caption is showing");
}

#[test]
fn captions_are_not_burned_in_unless_asked() {
    // Off by default: a sidecar can be switched off by the viewer and burned-in
    // text cannot, so burning in has to be deliberate.
    use mediacore_model::render::plan_at;
    let mut s = Session::new(Project::default());
    s.apply(Edit::SetCaptions { from: Captions::default(), to: sample() }).unwrap();
    let plan = plan_at(&s.project.timeline, secs(1.5));
    assert!(!plan.video.iter().any(|l| l.is_caption),
            "captions were burned in without being asked for");
}
