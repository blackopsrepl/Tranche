use rust_xlsxwriter::{DocProperties, ExcelDateTime, Format, Workbook, Worksheet, XlsxError};
use serde_json::{Value, json};
use tranche_core::report::Root;
use tranche_core::util::atomic_write;

const PR_COLUMNS: &[&str] = &[
    "number",
    "title",
    "author",
    "created",
    "draft",
    "category",
    "categories",
    "freshness",
    "risk",
    "security",
    "security_priority",
    "finished",
    "effort",
    "is_fix",
    "activity.head_moved",
    "activity.idle_since",
    "activity.idle_basis",
    "activity.thread_updated",
    "diffstat",
    "candidate",
    "senior",
    "followup",
    "related",
    "parked",
    "batches",
    "body",
    "body_truncated",
];

/// Write a workbook with filterable tables for the report's review surfaces.
pub(super) fn write(
    root: &Root,
    payload: &Value,
    report_binding: &str,
    repository: &str,
) -> Result<usize, String> {
    let document = super::export_data::document(payload, report_binding, repository);
    let path = root.docs_dir().join("data/report.xlsx");
    let mut workbook = Workbook::new();
    // Keep generated workbooks byte-stable for identical report inputs.
    let created = ExcelDateTime::from_ymd(2000, 1, 1).map_err(|error| error.to_string())?;
    workbook.set_properties(&DocProperties::new().set_creation_datetime(&created));
    write_report(&mut workbook, &document).map_err(|error| error.to_string())?;
    write_prs(&mut workbook, &document["pull_requests"]).map_err(|error| error.to_string())?;
    write_groups(&mut workbook, &document["groups"]).map_err(|error| error.to_string())?;
    write_batches(&mut workbook, &document["batches"]).map_err(|error| error.to_string())?;
    write_parked(&mut workbook, &document["parked"]).map_err(|error| error.to_string())?;
    let bytes = workbook
        .save_to_buffer()
        .map_err(|error| format!("cannot build {}: {error}", path.display()))?;
    atomic_write(&path, &bytes)
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    Ok(bytes.len())
}

fn write_report(workbook: &mut Workbook, document: &Value) -> Result<(), XlsxError> {
    let rows = vec![
        vec![json!("format"), document["format"].clone()],
        vec![json!("schema_version"), document["schema_version"].clone()],
        vec![json!("repository"), document["repository"].clone()],
        vec![json!("report_binding"), document["report_binding"].clone()],
        vec![
            json!("batches_available"),
            document["batches_available"].clone(),
        ],
        vec![
            json!("group_meaning"),
            document["groups"]["meaning"].clone(),
        ],
    ];
    write_table(workbook, "Report", &["field", "value"], &rows)
}

fn write_prs(workbook: &mut Workbook, prs: &Value) -> Result<(), XlsxError> {
    let rows: Vec<Vec<Value>> = prs
        .as_array()
        .into_iter()
        .flatten()
        .map(|pr| PR_COLUMNS.iter().map(|key| field(pr, key)).collect())
        .collect();
    write_table(workbook, "PRs", PR_COLUMNS, &rows)
}

fn write_groups(workbook: &mut Workbook, groups: &Value) -> Result<(), XlsxError> {
    let mut rows = Vec::new();
    for (index, group) in groups["confirmed_groups"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        rows.push(vec![
            json!("confirmed"),
            json!(format!("C{:03}", index + 1)),
            json!(member_numbers(group)),
            json!(""),
        ]);
    }
    for (index, group) in groups["review_groups"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        rows.push(vec![
            json!("review"),
            json!(format!("R{:03}", index + 1)),
            json!(member_numbers(&group["members"])),
            json!(compact(group)),
        ]);
    }
    for (index, pair) in groups["uncertain_pairs"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let members = [pair.get("a"), pair.get("b")]
            .into_iter()
            .flatten()
            .map(cell_text)
            .collect::<Vec<_>>()
            .join(", ");
        rows.push(vec![
            json!("uncertain"),
            json!(format!("U{:03}", index + 1)),
            json!(members),
            json!(compact(pair)),
        ]);
    }
    write_table(
        workbook,
        "Groups",
        &["kind", "group", "members", "details"],
        &rows,
    )
}

fn write_batches(workbook: &mut Workbook, batches: &Value) -> Result<(), XlsxError> {
    let columns = [
        "id",
        "count",
        "members",
        "security_members",
        "average_risk",
        "created",
        "review_prompt",
    ];
    let rows: Vec<Vec<Value>> = batches
        .as_array()
        .into_iter()
        .flatten()
        .map(|batch| columns.iter().map(|key| field(batch, key)).collect())
        .collect();
    write_table(workbook, "Batches", &columns, &rows)
}

fn write_parked(workbook: &mut Workbook, parked: &Value) -> Result<(), XlsxError> {
    let columns = [
        "number",
        "title",
        "author",
        "reasons",
        "unblock",
        "head_sha",
        "url",
        "created",
        "security_flag",
    ];
    let rows: Vec<Vec<Value>> = parked["members"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|member| columns.iter().map(|key| field(member, key)).collect())
        .collect();
    write_table(workbook, "Parked", &columns, &rows)
}

fn write_table(
    workbook: &mut Workbook,
    name: &str,
    headers: &[&str],
    rows: &[Vec<Value>],
) -> Result<(), XlsxError> {
    let worksheet = workbook.add_worksheet().set_name(name)?;
    let header_format = Format::new().set_bold();
    for (column, header) in headers.iter().enumerate() {
        let column = column as u16;
        worksheet.write_string_with_format(0, column, *header, &header_format)?;
        let width = match *header {
            "title" => 46.0,
            "body" | "review_prompt" | "unblock" | "details" => 60.0,
            "members" | "reasons" | "batches" => 28.0,
            _ => 18.0,
        };
        worksheet.set_column_width(column, width)?;
    }
    for (index, row) in rows.iter().enumerate() {
        for (column, value) in row.iter().enumerate() {
            write_cell(worksheet, index as u32 + 1, column as u16, value)?;
        }
    }
    worksheet.autofilter(0, 0, rows.len() as u32, headers.len() as u16 - 1)?;
    worksheet.set_freeze_panes(1, 0)?;
    Ok(())
}

fn write_cell(
    worksheet: &mut Worksheet,
    row: u32,
    column: u16,
    value: &Value,
) -> Result<(), XlsxError> {
    match value {
        Value::Null => Ok(()),
        Value::Bool(value) => {
            worksheet.write_boolean(row, column, *value)?;
            Ok(())
        }
        Value::Number(value) => {
            worksheet.write_number(row, column, value.as_f64().unwrap_or_default())?;
            Ok(())
        }
        Value::String(value) => {
            worksheet.write_string(row, column, value)?;
            Ok(())
        }
        Value::Array(_) | Value::Object(_) => {
            worksheet.write_string(row, column, cell_text(value))?;
            Ok(())
        }
    }
}

fn field(record: &Value, path: &str) -> Value {
    path.split('.').fold(record.clone(), |value, key| {
        value.get(key).cloned().unwrap_or(Value::Null)
    })
}

fn member_numbers(value: &Value) -> String {
    value
        .as_array()
        .into_iter()
        .flatten()
        .map(cell_text)
        .collect::<Vec<_>>()
        .join(", ")
}

fn cell_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Array(items) => items.iter().map(cell_text).collect::<Vec<_>>().join(", "),
        Value::Object(object) => {
            if let (Some(id), Some(count)) = (object.get("id"), object.get("count")) {
                format!("{} ({})", cell_text(id), cell_text(count))
            } else {
                compact(value)
            }
        }
        _ => value.to_string(),
    }
}

fn compact(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}
