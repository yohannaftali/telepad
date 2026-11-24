//! Export and import of table data (#139): SQL `INSERT`s, CSV and Excel.
//!
//! Export writes rows batch by batch (the caller pages through the table)
//! into `<file>.telepad-part`, renamed when complete. Import reads a file
//! into rows, maps its columns to the table's and builds one `INSERT` per
//! row with bound values; a SQL file is accepted only when every statement
//! is an `INSERT` into the chosen table.

use std::io::Write;
use std::path::{Path, PathBuf};

use super::sql::{self, quote_ident, quote_str, table_path};
use super::{Cell, Column, Engine, Statement, TableRef};

/// File format of an export / import.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Format {
    #[default]
    Csv,
    Sql,
    Xlsx,
}

impl Format {
    pub const ALL: [Format; 3] = [Format::Csv, Format::Sql, Format::Xlsx];
    pub fn title(self) -> &'static str {
        match self {
            Format::Csv => "CSV",
            Format::Sql => "SQL (INSERT)",
            Format::Xlsx => "Excel (.xlsx)",
        }
    }
    pub fn ext(self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::Sql => "sql",
            Format::Xlsx => "xlsx",
        }
    }
    /// By file name, for import.
    pub fn of_path(p: &Path) -> Option<Format> {
        match p
            .extension()?
            .to_string_lossy()
            .to_ascii_lowercase()
            .as_str()
        {
            "csv" | "tsv" | "txt" => Some(Format::Csv),
            "sql" => Some(Format::Sql),
            "xlsx" | "xlsm" | "xls" | "ods" => Some(Format::Xlsx),
            _ => None,
        }
    }
}

/// Which rows an export takes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Scope {
    #[default]
    Filtered,
    Table,
    Page,
}

impl Scope {
    pub const ALL: [Scope; 3] = [Scope::Filtered, Scope::Table, Scope::Page];
    pub fn title(self) -> &'static str {
        match self {
            Scope::Filtered => "Filtered rows (all pages)",
            Scope::Table => "Whole table",
            Scope::Page => "This page",
        }
    }
}

/// What the user chose in the Export dialog.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExportSpec {
    pub format: Format,
    pub scope: Scope,
    /// CSV field separator.
    pub semicolon: bool,
    /// SQL: a `CREATE TABLE` first (the server's own text, when known).
    pub create: Option<String>,
    /// Columns written, in order (the shown ones, or all).
    pub columns: Vec<String>,
}

/// A spreadsheet would run a text cell that starts like a formula (#111):
/// prefix it with `'`. Numbers (also negative ones) stay as they are.
fn defuse(s: &str) -> std::borrow::Cow<'_, str> {
    if s.starts_with(['=', '+', '-', '@', '\t', '\r']) && s.trim().parse::<f64>().is_err() {
        format!("'{s}").into()
    } else {
        s.into()
    }
}

/// Writes an export. `rows` may be called many times (one page each).
pub struct Exporter {
    part: PathBuf,
    target: PathBuf,
    engine: Engine,
    table: TableRef,
    spec: ExportSpec,
    /// Positions of the written columns in the grid's columns.
    pick: Vec<usize>,
    out: Out,
    /// Rows written, and values that were cut to the grid's cell size.
    pub rows: u64,
    pub cut: u64,
}

enum Out {
    Csv(Box<csv::Writer<std::fs::File>>),
    Sql(std::io::BufWriter<std::fs::File>),
    Xlsx(Box<rust_xlsxwriter::Workbook>, u32),
}

impl Exporter {
    /// Start an export of `grid_columns` into `target`.
    pub fn create(
        target: &Path,
        engine: Engine,
        table: &TableRef,
        grid_columns: &[String],
        spec: ExportSpec,
    ) -> Result<Self, String> {
        let pick: Vec<usize> = spec
            .columns
            .iter()
            .filter_map(|c| grid_columns.iter().position(|g| g == c))
            .collect();
        if pick.is_empty() {
            return Err("No columns to export".into());
        }
        let names: Vec<&str> = pick.iter().map(|&i| grid_columns[i].as_str()).collect();
        let part = PathBuf::from(format!("{}.telepad-part", target.display()));
        let err = |e: &dyn std::fmt::Display| format!("{}: {e}", target.display());
        let out = match spec.format {
            Format::Csv => {
                let f = std::fs::File::create(&part).map_err(|e| err(&e))?;
                let mut w = csv::WriterBuilder::new()
                    .delimiter(if spec.semicolon { b';' } else { b',' })
                    .from_writer(f);
                w.write_record(&names).map_err(|e| err(&e))?;
                Out::Csv(Box::new(w))
            }
            Format::Sql => {
                let f = std::fs::File::create(&part).map_err(|e| err(&e))?;
                let mut w = std::io::BufWriter::new(f);
                writeln!(w, "-- Telepad export of {}\n", table_path(engine, table))
                    .map_err(|e| err(&e))?;
                if let Some(c) = spec.create.as_ref().filter(|c| !c.trim().is_empty()) {
                    writeln!(w, "{};\n", c.trim().trim_end_matches(';')).map_err(|e| err(&e))?;
                }
                Out::Sql(w)
            }
            Format::Xlsx => {
                let mut wb = rust_xlsxwriter::Workbook::new();
                let bold = rust_xlsxwriter::Format::new().set_bold();
                let ws = wb
                    .add_worksheet_with_constant_memory()
                    .map_err(|e| err(&e))?;
                let sheet: String = table.table.chars().take(31).collect();
                let _ = ws.set_name(sheet.replace(['[', ']', ':', '*', '?', '/', '\\'], "_"));
                for (c, n) in names.iter().enumerate() {
                    ws.write_string_with_format(0, c as u16, *n, &bold)
                        .map_err(|e| err(&e))?;
                }
                Out::Xlsx(Box::new(wb), 1)
            }
        };
        Ok(Self {
            part,
            target: target.to_path_buf(),
            engine,
            table: table.clone(),
            spec,
            pick,
            out,
            rows: 0,
            cut: 0,
        })
    }

    /// Write one batch of rows (cells in the grid's column order).
    pub fn rows(&mut self, rows: &[Vec<Cell>], grid_columns: &[String]) -> Result<(), String> {
        let err = |e: &dyn std::fmt::Display| e.to_string();
        for row in rows {
            let cells: Vec<&Cell> = self.pick.iter().filter_map(|&i| row.get(i)).collect();
            self.cut += cells
                .iter()
                .filter(|c| matches!(c, Cell::Long(..) | Cell::Bytes(..)))
                .count() as u64;
            match &mut self.out {
                Out::Csv(w) => {
                    let v: Vec<String> = cells
                        .iter()
                        .map(|c| {
                            c.as_value()
                                .map(|s| defuse(s).into_owned())
                                .unwrap_or_default()
                        })
                        .collect();
                    w.write_record(&v).map_err(|e| err(&e))?;
                }
                Out::Sql(w) => {
                    let names: Vec<String> = self
                        .pick
                        .iter()
                        .map(|&i| quote_ident(self.engine, &grid_columns[i]))
                        .collect();
                    let vals: Vec<String> = cells
                        .iter()
                        .map(|c| match c.as_value() {
                            Some(v) => quote_str(self.engine, v),
                            None => "NULL".into(),
                        })
                        .collect();
                    writeln!(
                        w,
                        "INSERT INTO {} ({}) VALUES ({});",
                        table_path(self.engine, &self.table),
                        names.join(", "),
                        vals.join(", ")
                    )
                    .map_err(|e| err(&e))?;
                }
                Out::Xlsx(wb, next) => {
                    let ws = wb.worksheet_from_index(0).map_err(|e| err(&e))?;
                    for (c, cell) in cells.iter().enumerate() {
                        let Some(v) = cell.as_value() else { continue };
                        // Numbers as numbers (not codes with leading zeros).
                        let num = v.trim().parse::<f64>().ok().filter(|n| {
                            n.is_finite()
                                && !(v.starts_with('0') && v.len() > 1 && !v.starts_with("0."))
                        });
                        match num {
                            Some(n) => ws.write_number(*next, c as u16, n),
                            None => ws.write_string(*next, c as u16, v),
                        }
                        .map_err(|e| err(&e))?;
                    }
                    *next += 1;
                }
            }
            self.rows += 1;
        }
        Ok(())
    }

    /// Finish the file and move it into place.
    pub fn finish(self) -> Result<(u64, u64), String> {
        let err = |e: &dyn std::fmt::Display| format!("{}: {e}", self.target.display());
        match self.out {
            Out::Csv(mut w) => w.flush().map_err(|e| err(&e))?,
            Out::Sql(mut w) => w.flush().map_err(|e| err(&e))?,
            Out::Xlsx(mut wb, _) => wb.save(&self.part).map_err(|e| err(&e))?,
        }
        if self.target.exists() {
            std::fs::remove_file(&self.target).map_err(|e| err(&e))?;
        }
        std::fs::rename(&self.part, &self.target).map_err(|e| err(&e))?;
        Ok((self.rows, self.cut))
    }

    /// Give up: the part file goes.
    pub fn abort(self) {
        let part = self.part.clone();
        drop(self);
        let _ = std::fs::remove_file(part);
    }

    pub fn spec(&self) -> &ExportSpec {
        &self.spec
    }
}

/// A file read for import: its column names and rows (`None` = NULL / empty
/// cell), or SQL statements.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportFile {
    pub format: Format,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    /// SQL: the INSERT statements for the table, and the ones refused
    /// (anything else, or another table).
    pub statements: Vec<String>,
    pub refused: Vec<String>,
}

/// Most rows one import takes (it runs in one transaction).
pub const MAX_IMPORT_ROWS: usize = 200_000;

/// Read a file for import into `table`.
pub fn read_import(path: &Path, engine: Engine, table: &TableRef) -> Result<ImportFile, String> {
    let format = Format::of_path(path)
        .ok_or_else(|| "Choose a .csv, .tsv, .sql, .xlsx, .xls or .ods file".to_string())?;
    let err = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    match format {
        Format::Csv => {
            let text = std::fs::read_to_string(path).map_err(|e| err(&e))?;
            let first = text.lines().next().unwrap_or("");
            let delim = (*b"\t;,")
                .into_iter()
                .max_by_key(|d| first.bytes().filter(|b| b == d).count())
                .unwrap_or(b',');
            let mut r = csv::ReaderBuilder::new()
                .delimiter(delim)
                .flexible(true)
                .from_reader(text.trim_start_matches('\u{feff}').as_bytes());
            let headers: Vec<String> = r
                .headers()
                .map_err(|e| err(&e))?
                .iter()
                .map(|h| h.trim().to_string())
                .collect();
            let mut rows = Vec::new();
            for rec in r.records() {
                let rec = rec.map_err(|e| err(&e))?;
                rows.push(
                    rec.iter()
                        .map(|v| (!v.is_empty()).then(|| v.to_string()))
                        .collect(),
                );
                if rows.len() > MAX_IMPORT_ROWS {
                    return Err(format!("More than {MAX_IMPORT_ROWS} rows; split the file"));
                }
            }
            Ok(ImportFile {
                format,
                headers,
                rows,
                ..Default::default()
            })
        }
        Format::Xlsx => {
            use calamine::{Reader, open_workbook_auto};
            let mut wb = open_workbook_auto(path).map_err(|e| err(&e))?;
            let range = wb
                .worksheet_range_at(0)
                .ok_or_else(|| "The workbook has no sheet".to_string())?
                .map_err(|e| err(&e))?;
            let mut it = range.rows();
            let headers: Vec<String> = it
                .next()
                .map(|r| r.iter().map(|c| c.to_string().trim().to_string()).collect())
                .unwrap_or_default();
            let mut rows = Vec::new();
            for r in it {
                rows.push(
                    r.iter()
                        .map(|c| {
                            let s = c.to_string();
                            (!s.is_empty()).then_some(s)
                        })
                        .collect(),
                );
                if rows.len() > MAX_IMPORT_ROWS {
                    return Err(format!("More than {MAX_IMPORT_ROWS} rows; split the file"));
                }
            }
            Ok(ImportFile {
                format,
                headers,
                rows,
                ..Default::default()
            })
        }
        Format::Sql => {
            let text = std::fs::read_to_string(path).map_err(|e| err(&e))?;
            let (statements, refused) = sql_inserts(engine, &text, table);
            Ok(ImportFile {
                format,
                statements,
                refused,
                ..Default::default()
            })
        }
    }
}

/// The statements of a SQL file that are `INSERT INTO` the chosen table
/// (by its name, quoted or not, with or without database / schema); every
/// other statement is refused (#139: import adds rows, nothing else).
pub fn sql_inserts(engine: Engine, text: &str, table: &TableRef) -> (Vec<String>, Vec<String>) {
    let mut ok = Vec::new();
    let mut refused = Vec::new();
    for r in sql::split_statements(engine, text) {
        let st = text[r].trim();
        if st.is_empty() {
            continue;
        }
        if insert_target(st).is_some_and(|t| t.eq_ignore_ascii_case(&table.table)) {
            ok.push(st.to_string());
        } else {
            refused.push(st.chars().take(120).collect());
        }
    }
    (ok, refused)
}

/// The table name (last part, unquoted) of `INSERT INTO a.b (…)`.
fn insert_target(st: &str) -> Option<String> {
    let body = strip_comments(st);
    let mut words = body.split_whitespace();
    if !words.next()?.eq_ignore_ascii_case("insert") {
        return None;
    }
    let mut w = words.next()?;
    if w.eq_ignore_ascii_case("ignore") {
        w = words.next()?;
    }
    if !w.eq_ignore_ascii_case("into") {
        return None;
    }
    let name = words.next()?;
    let name = name.split('(').next()?;
    let last = name.rsplit('.').next()?;
    Some(last.trim_matches(['`', '"', '[', ']']).to_string())
}

fn strip_comments(st: &str) -> String {
    st.lines()
        .map(|l| l.split("--").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ")
}

/// File column → table column (by name, case ignored); `None` = not imported.
pub fn auto_map(headers: &[String], columns: &[Column]) -> Vec<Option<String>> {
    headers
        .iter()
        .map(|h| {
            columns
                .iter()
                .find(|c| c.name.eq_ignore_ascii_case(h.trim()))
                .map(|c| c.name.clone())
        })
        .collect()
}

/// One `INSERT` per row with the mapped columns, values bound (never pasted
/// into the SQL).
pub fn import_statements(
    engine: Engine,
    table: &TableRef,
    columns: &[Column],
    map: &[Option<String>],
    rows: &[Vec<Option<String>>],
) -> Vec<Statement> {
    rows.iter()
        .map(|row| {
            let values: Vec<(String, Option<String>)> = map
                .iter()
                .enumerate()
                .filter_map(|(i, m)| {
                    m.as_ref()
                        .map(|c| (c.clone(), row.get(i).cloned().flatten()))
                })
                .collect();
            sql::insert_row(engine, table, columns, &values)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> TableRef {
        TableRef {
            database: "shop".into(),
            schema: String::new(),
            table: "items".into(),
        }
    }

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("telepad-io-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn rows() -> Vec<Vec<Cell>> {
        vec![
            vec![
                Cell::Text("1".into()),
                Cell::Text("Widget, large".into()),
                Cell::Text("-5".into()),
            ],
            vec![
                Cell::Text("2".into()),
                Cell::Text("=HYPERLINK(1)".into()),
                Cell::Null,
            ],
        ]
    }

    fn cols() -> Vec<String> {
        vec!["id".into(), "name".into(), "qty".into()]
    }

    #[test]
    fn exports_csv_sql_and_xlsx() {
        let d = dir("export");
        let spec = |format| ExportSpec {
            format,
            columns: vec!["id".into(), "name".into(), "qty".into()],
            create: Some("CREATE TABLE items (id int)".into()),
            ..Default::default()
        };
        // CSV: quoted, formulas defused, numbers kept.
        let p = d.join("a.csv");
        let mut e = Exporter::create(&p, Engine::MySql, &t(), &cols(), spec(Format::Csv)).unwrap();
        e.rows(&rows(), &cols()).unwrap();
        assert_eq!(e.finish().unwrap(), (2, 0));
        let text = std::fs::read_to_string(&p).unwrap();
        assert_eq!(
            text,
            "id,name,qty\n1,\"Widget, large\",-5\n2,'=HYPERLINK(1),\n"
        );
        assert!(!d.join("a.csv.telepad-part").exists());
        // SQL: quoted names and values, NULL, the CREATE TABLE first.
        let p = d.join("a.sql");
        let mut e = Exporter::create(&p, Engine::MySql, &t(), &cols(), spec(Format::Sql)).unwrap();
        e.rows(&rows(), &cols()).unwrap();
        e.finish().unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("CREATE TABLE items (id int);"));
        assert!(text.contains(
            "INSERT INTO `shop`.`items` (`id`, `name`, `qty`) VALUES ('2', '=HYPERLINK(1)', NULL);"
        ));
        // Excel: readable back with the same values.
        let p = d.join("a.xlsx");
        let mut e = Exporter::create(&p, Engine::MySql, &t(), &cols(), spec(Format::Xlsx)).unwrap();
        e.rows(&rows(), &cols()).unwrap();
        e.finish().unwrap();
        let back = read_import(&p, Engine::MySql, &t()).unwrap();
        assert_eq!(back.headers, cols());
        assert_eq!(back.rows.len(), 2);
        assert_eq!(back.rows[0][1].as_deref(), Some("Widget, large"));
        assert_eq!(back.rows[0][2].as_deref(), Some("-5"));
        assert_eq!(back.rows[1][2], None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn imports_csv_with_a_column_map_and_bound_values() {
        let d = dir("import");
        let p = d.join("in.csv");
        std::fs::write(&p, "\u{feff}ID;Name;extra\n7;O'Brien;x\n8;;y\n").unwrap();
        let f = read_import(&p, Engine::MySql, &t()).unwrap();
        assert_eq!(f.headers, ["ID", "Name", "extra"]);
        assert_eq!(f.rows[1][1], None, "an empty cell is NULL");
        let table_cols = vec![
            Column {
                name: "id".into(),
                ..Default::default()
            },
            Column {
                name: "name".into(),
                ..Default::default()
            },
        ];
        let map = auto_map(&f.headers, &table_cols);
        assert_eq!(map, [Some("id".into()), Some("name".into()), None]);
        let st = import_statements(Engine::MySql, &t(), &table_cols, &map, &f.rows);
        assert_eq!(
            st[0].sql,
            "INSERT INTO `shop`.`items` (`id`, `name`) VALUES (?, ?)"
        );
        assert_eq!(st[0].params, [Some("7".into()), Some("O'Brien".into())]);
        assert_eq!(st[1].params, [Some("8".into()), None]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn sql_import_takes_only_inserts_into_the_table() {
        let text = "-- dump\nINSERT INTO `items` (id) VALUES (1),(2);\n\
                    insert into shop.items values (3);\n\
                    INSERT INTO other (id) VALUES (9);\n\
                    DROP TABLE items;\n\
                    UPDATE items SET id = 0;";
        let (ok, refused) = sql_inserts(Engine::MySql, text, &t());
        assert_eq!(ok.len(), 2);
        assert_eq!(refused.len(), 3);
        assert!(refused.iter().any(|r| r.starts_with("DROP TABLE")));
    }
}
