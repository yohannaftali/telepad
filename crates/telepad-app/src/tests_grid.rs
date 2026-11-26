//! Behaviour tests for #153: the table component's state (sort, filter row,
//! Filter by, Clear All Filters), the rows an export takes and the table
//! menu messages.

use std::time::Instant;

use crate::grid::{
    self, Dir, Format, PDF_ASK_ABOVE, TableData, TableMsg, TableState, compare, data, filter_match,
    filter_value, value, view,
};
use crate::pager::Go;
use crate::*;

fn cells() -> Vec<Vec<String>> {
    [
        ["web-02", "192.0.2.10", "75%", "1.5 GiB"],
        ["db-01", "192.0.2.9", "5%", "980 MiB"],
        ["web-01", "198.51.100.7", "", "2 GiB"],
        ["cache-01", "192.0.2.100", "40%", "512 MiB"],
    ]
    .iter()
    .map(|r| r.iter().map(|c| c.to_string()).collect())
    .collect()
}

#[test]
fn header_click_sorts_ascending_descending_then_own_order() {
    let c = cells();
    let mut st = TableState::default();
    assert_eq!(view(&c, &st), vec![0, 1, 2, 3]);
    st.click(1);
    // Addresses in natural order: .9 before .10 before .100.
    assert_eq!(view(&c, &st), vec![1, 0, 3, 2]);
    st.click(1);
    assert_eq!(st.sort, Some((1, Dir::Desc)));
    assert_eq!(view(&c, &st), vec![2, 3, 0, 1]);
    st.click(1);
    assert_eq!(st.sort, None);
    assert_eq!(view(&c, &st), vec![0, 1, 2, 3]);
}

#[test]
fn numbers_with_units_sort_by_value_and_empty_cells_go_last() {
    let c = cells();
    let mut st = TableState::default();
    st.click(2);
    assert_eq!(view(&c, &st), vec![1, 3, 0, 2], "5% 40% 75% then empty");
    st.click(2);
    assert_eq!(
        view(&c, &st),
        vec![0, 3, 1, 2],
        "empty stays last when descending"
    );
    let mut st = TableState::default();
    st.click(3);
    assert_eq!(
        view(&c, &st),
        vec![3, 1, 0, 2],
        "512 MiB < 980 MiB < 1.5 GiB < 2 GiB"
    );
    assert_eq!(value("2h 5m ago"), Some(7500.0));
    assert_eq!(value("12s ago"), Some(12.0));
    assert_eq!(value("192.0.2.1"), None);
    assert_eq!(compare("b", "A"), std::cmp::Ordering::Greater);
}

#[test]
fn filter_boxes_use_the_wtbapp_rules_with_units() {
    assert!(filter_match("75%", ">50"));
    assert!(!filter_match("40%", ">50"));
    assert!(filter_match("1.5 GiB", ">1 GiB"));
    assert!(filter_match("Web-01", "web"));
    assert!(filter_match("", "NULL"));
    assert!(filter_match("x", "!NULL"));
    assert!(filter_match("db-01", "=DB-01"));
    assert!(!filter_match("db-01", "!db-01"));
    // Times as text.
    assert!(filter_match("2026-11-24 10:00", ">2026-11-20"));
    let c = cells();
    let mut st = TableState::default();
    *st.filter_mut(0) = "web".into();
    assert_eq!(view(&c, &st), vec![0, 2]);
    *st.filter_mut(2) = ">50".into();
    assert_eq!(view(&c, &st), vec![0]);
}

#[test]
fn filter_by_fills_the_box_and_clear_empties_every_box() {
    let c = cells();
    let mut st = TableState::default();
    st.pager.per_page = 1;
    st.pager.go(Go::Last, 4);
    st.filter_by(1, "192.0.2.9");
    assert_eq!(st.filters[1], "192.0.2.9");
    assert_eq!(st.pager.page, 0, "a new filter starts at page 1");
    assert_eq!(view(&c, &st), vec![1]);
    st.filter_by(3, "512 MiB");
    assert!(view(&c, &st).is_empty());
    assert!(st.clear_filters());
    assert!(!st.filtered());
    assert_eq!(view(&c, &st).len(), 4);
    assert!(!st.clear_filters(), "nothing left to clear");
    // Values that read as an operator are matched exactly.
    assert_eq!(filter_value(">5"), "=>5");
    assert_eq!(filter_value("null"), "=null");
    assert_eq!(filter_value(" plain "), "plain");
}

#[test]
fn an_export_takes_every_filtered_sorted_row_of_every_page() {
    let c = cells();
    let mut st = TableState::default();
    st.pager.per_page = 1;
    *st.filter_mut(0) = "-01".into();
    st.click(0);
    let order = view(&c, &st);
    // One row per page on screen, but the export has all three.
    let (s, e, pages) = st.pager.range(order.len());
    assert_eq!((e - s, pages), (1, 3));
    let cols = vec![
        ("Host".to_string(), true),
        ("Address".to_string(), true),
        ("CPU".to_string(), false),
        ("Size".to_string(), true),
    ];
    let t = data("Hosts", &cols, &c, &order);
    assert_eq!(t.columns, vec!["Host", "Address", "Size"]);
    assert_eq!(
        t.rows,
        vec![
            vec!["cache-01", "192.0.2.100", "512 MiB"],
            vec!["db-01", "192.0.2.9", "980 MiB"],
            vec!["web-01", "198.51.100.7", "2 GiB"],
        ]
    );
}

fn table(n: usize) -> TableData {
    TableData {
        title: "Ports".into(),
        columns: vec!["Port".into()],
        rows: (0..n).map(|i| vec![i.to_string()]).collect(),
    }
}

#[test]
fn copy_as_json_goes_to_the_clipboard_and_files_to_the_save_dialog() {
    let mut s = State::default();
    let now = Instant::now();
    let fx = update(
        &mut s,
        Msg::Table(TableMsg::Export {
            data: table(2),
            format: Format::Json,
        }),
        now,
    );
    match fx.as_slice() {
        [Effect::CopyText { text, notice }] => {
            assert!(text.starts_with('['));
            assert_eq!(notice, "Copied 2 rows as JSON");
        }
        other => panic!("{other:?}"),
    }
    for f in [Format::Csv, Format::Xlsx, Format::Pdf] {
        let fx = update(
            &mut s,
            Msg::Table(TableMsg::Export {
                data: table(3),
                format: f,
            }),
            now,
        );
        assert!(
            matches!(fx.as_slice(), [Effect::ExportTable { format, data }] if *format == f && data.rows.len() == 3)
        );
    }
    update(
        &mut s,
        Msg::Table(TableMsg::Exported(Ok((3, "ports.csv".into())))),
        now,
    );
    assert_eq!(
        s.model.message.as_ref().map(|m| m.0.as_str()),
        Some("Exported 3 rows to ports.csv")
    );
}

#[test]
fn a_long_pdf_asks_first() {
    let mut s = State::default();
    let now = Instant::now();
    let big = table(PDF_ASK_ABOVE + 1);
    let fx = update(
        &mut s,
        Msg::Table(TableMsg::Export {
            data: big.clone(),
            format: Format::Pdf,
        }),
        now,
    );
    assert!(fx.is_empty());
    assert!(s.model.table_ask.is_some());
    let fx = update(&mut s, Msg::Table(TableMsg::Answer(false)), now);
    assert!(fx.is_empty() && s.model.table_ask.is_none());
    update(
        &mut s,
        Msg::Table(TableMsg::Export {
            data: big,
            format: Format::Pdf,
        }),
        now,
    );
    let fx = update(&mut s, Msg::Table(TableMsg::Answer(true)), now);
    assert!(matches!(
        fx.as_slice(),
        [Effect::ExportTable {
            format: Format::Pdf,
            ..
        }]
    ));
    assert!(s.model.table_ask.is_none());
    // CSV of the same size does not ask.
    let fx = update(
        &mut s,
        Msg::Table(TableMsg::Export {
            data: table(PDF_ASK_ABOVE + 1),
            format: Format::Csv,
        }),
        now,
    );
    assert_eq!(fx.len(), 1);
    let _ = grid::natural("a", "b");
}

#[test]
fn the_value_dropdown_lists_each_value_once_in_natural_order() {
    // #170
    let col = ["web10", "web2", " Web2 ", "", "db1", "web1"];
    let v = grid::distinct_values(col, grid::VALUES_CAP);
    assert_eq!(v, ["db1", "web1", "web2", "web10", ""]);
    // The cap counts distinct values in row order.
    assert_eq!(grid::distinct_values(col, 2), ["web2", "web10"]);
    // Search: contains, any case.
    assert_eq!(grid::search_values(&v, "WEB1"), ["web1", "web10"]);
    assert_eq!(grid::search_values(&v, "").len(), 5);
    // Picking a value filters to exactly that value; an empty cell is NULL.
    assert_eq!(grid::exact_filter(" web1 "), "=web1");
    assert_eq!(grid::exact_filter(""), "NULL");
    assert!(filter_match("web1", &grid::exact_filter("web1")));
    assert!(!filter_match("web10", &grid::exact_filter("web1")));
    assert!(filter_match("", &grid::exact_filter("")));
    assert!(!filter_match("x", &grid::exact_filter("")));
}

#[test]
fn the_value_dropdown_follows_the_other_filters_only() {
    // #170: column 1's list comes from rows that pass column 0's filter,
    // whatever column 1's own filter says.
    let rows = [["Office", "web1"], ["Office", "web2"], ["Lab", "db1"]];
    let filters = ["=Office".to_string(), "=web1".to_string()];
    let values: Vec<&str> = rows
        .iter()
        .filter(|r| grid::row_matches_except(&|c| r[c].to_string(), &filters, 1))
        .map(|r| r[1])
        .collect();
    assert_eq!(
        grid::distinct_values(values, grid::VALUES_CAP),
        ["web1", "web2"]
    );
}
