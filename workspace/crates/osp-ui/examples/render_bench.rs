use std::hint::black_box;
use std::time::{Duration, Instant};

use osp_core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
use osp_core::output_model::{OutputItems, OutputMeta, OutputResult};
use osp_core::row::Row;
use osp_ui::messages::{GroupedRenderOptions, MessageBuffer, MessageLevel};
use osp_ui::{RenderRuntime, RenderSettings, render_output, style::StyleOverrides};

fn main() {
    let iterations = std::env::args()
        .nth(1)
        .and_then(|raw| raw.parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(200);

    let table_rich_settings = rich_settings(OutputFormat::Table);
    let table_plain_settings = plain_settings(OutputFormat::Table);
    let json_settings = rich_settings(OutputFormat::Json);
    let mreg_settings = rich_settings(OutputFormat::Mreg);

    let table_output = sample_table_output();
    let json_output = sample_json_output();
    let mreg_output = sample_mreg_output();
    let messages = sample_messages();

    println!("osp-ui render bench ({iterations} iterations)");
    print_case("table.rich", iterations, || {
        black_box(render_output(
            black_box(&table_output),
            black_box(&table_rich_settings),
        ))
    });
    print_case("table.plain", iterations, || {
        black_box(render_output(
            black_box(&table_output),
            black_box(&table_plain_settings),
        ))
    });
    print_case("json.rich", iterations, || {
        black_box(render_output(
            black_box(&json_output),
            black_box(&json_settings),
        ))
    });
    print_case("mreg.rich", iterations, || {
        black_box(render_output(
            black_box(&mreg_output),
            black_box(&mreg_settings),
        ))
    });
    print_case("messages.grouped", iterations, || {
        let resolved = table_rich_settings.resolve_render_settings();
        black_box(messages.render_grouped_with_options(GroupedRenderOptions {
            max_level: MessageLevel::Info,
            color: resolved.color,
            unicode: resolved.unicode,
            width: resolved.width,
            theme: &resolved.theme,
            layout: osp_ui::messages::MessageLayout::Grouped,
            chrome_frame: resolved.chrome_frame,
            style_overrides: StyleOverrides::default(),
        }))
    });
}

fn print_case(name: &str, iterations: u32, mut f: impl FnMut() -> String) {
    let start = Instant::now();
    let mut bytes = 0usize;
    for _ in 0..iterations {
        bytes = bytes.saturating_add(f().len());
    }
    let elapsed = start.elapsed();
    println!(
        "{name:16} total={:>8}  avg={:>10} us/op  bytes={bytes}",
        format_duration(elapsed),
        micros_per_op(elapsed, iterations),
    );
}

fn micros_per_op(elapsed: Duration, iterations: u32) -> String {
    let per_op = elapsed.as_secs_f64() * 1_000_000.0 / f64::from(iterations.max(1));
    format!("{per_op:.2}")
}

fn format_duration(duration: Duration) -> String {
    let millis = duration.as_secs_f64() * 1_000.0;
    format!("{millis:.2} ms")
}

fn rich_settings(format: OutputFormat) -> RenderSettings {
    RenderSettings {
        format,
        mode: RenderMode::Rich,
        color: ColorMode::Always,
        unicode: UnicodeMode::Always,
        width: Some(96),
        runtime: RenderRuntime {
            stdout_is_tty: true,
            terminal: Some("xterm-256color".to_string()),
            no_color: false,
            width: Some(96),
            locale_utf8: Some(true),
        },
        ..RenderSettings::test_plain(format)
    }
}

fn plain_settings(format: OutputFormat) -> RenderSettings {
    RenderSettings {
        format,
        mode: RenderMode::Plain,
        color: ColorMode::Never,
        unicode: UnicodeMode::Never,
        width: Some(96),
        runtime: RenderRuntime {
            stdout_is_tty: false,
            terminal: Some("dumb".to_string()),
            no_color: true,
            width: Some(96),
            locale_utf8: Some(false),
        },
        ..RenderSettings::test_plain(format)
    }
}

fn sample_table_output() -> OutputResult {
    let rows = (0..120)
        .map(|index| {
            row([
                ("uid", format!("user-{index:03}").into()),
                (
                    "status",
                    if index % 3 == 0 { "ready" } else { "pending" }.into(),
                ),
                ("count", serde_json::Value::from((index * 7) as i64)),
                ("owner", format!("team-{}", (index % 8) + 1).into()),
                (
                    "note",
                    format!("payload slice {index} with a medium-width description").into(),
                ),
            ])
        })
        .collect::<Vec<Row>>();

    OutputResult {
        items: OutputItems::Rows(rows),
        meta: OutputMeta::default(),
    }
}

fn sample_json_output() -> OutputResult {
    let rows = (0..32)
        .map(|index| {
            row([
                ("uid", format!("user-{index:03}").into()),
                (
                    "meta",
                    serde_json::json!({
                        "enabled": index % 2 == 0,
                        "tags": ["alpha", "beta", format!("role-{index}")],
                        "limits": { "cpu": index + 1, "memory": (index + 1) * 4 },
                    }),
                ),
            ])
        })
        .collect::<Vec<Row>>();

    OutputResult {
        items: OutputItems::Rows(rows),
        meta: OutputMeta::default(),
    }
}

fn sample_mreg_output() -> OutputResult {
    OutputResult {
        items: OutputItems::Rows(vec![row([
            ("uid", "ops-admin".into()),
            ("enabled", true.into()),
            (
                "groups",
                serde_json::json!(["wheel", "ops", "netadmins", "dba"]),
            ),
            (
                "services",
                serde_json::json!([
                    {"name": "vmware", "status": "ok"},
                    {"name": "nrec", "status": "ok"},
                    {"name": "ldap", "status": "degraded"}
                ]),
            ),
        ])]),
        meta: OutputMeta::default(),
    }
}

fn sample_messages() -> MessageBuffer {
    let mut messages = MessageBuffer::default();
    messages.error("provider returned an unexpected transient failure");
    messages.warning("falling back to cached capabilities");
    messages.info("reloading render profile after config mutation");
    messages
}

fn row<const N: usize>(entries: [(&str, serde_json::Value); N]) -> Row {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}
