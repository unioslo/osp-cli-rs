use std::hint::black_box;
use std::io::Write;
use std::time::{Duration, Instant};

use osp_core::output::OutputFormat;
use osp_core::output_model::OutputResult;
use osp_core::row::Row;
use osp_dsl::{apply_pipeline, execute_pipeline_streaming};
use osp_ui::{RenderSettings, render_output};
use serde_json::json;

fn main() {
    let settings = RenderSettings::test_plain(OutputFormat::Table);
    let stages = vec![
        "alice".to_string(),
        "P uid,mail,profile.department".to_string(),
    ];
    let cases = [
        ("small", 25usize, 400u32),
        ("medium", 500usize, 120u32),
        ("big", 20_000usize, 12u32),
    ];

    println!("osp-cli dsl+render benchmark (table plain, single write)");
    println!("pipeline: {}", stages.join(" | "));

    for (name, row_count, iterations) in cases {
        let rows = bench_rows(row_count);
        let eager = apply_pipeline(rows.clone(), &stages).expect("eager pipeline should pass");
        let streaming = execute_pipeline_streaming(rows.clone(), &stages)
            .expect("streaming pipeline should pass");
        assert_eq!(streaming, eager, "streaming output should match eager");

        let rendered = render_output(&streaming, &settings);
        println!(
            "\n{name:6} rows={row_count:<6} rendered_bytes={} iterations={iterations}",
            rendered.len()
        );

        print_total_case("eager+render+write", iterations, || {
            eager_end_to_end(
                black_box(rows.clone()),
                black_box(&stages),
                black_box(&settings),
            )
        });
        print_total_case("stream+render+write", iterations, || {
            streaming_end_to_end(
                black_box(rows.clone()),
                black_box(&stages),
                black_box(&settings),
            )
        });
        print_phase_case("eager first/all", iterations, || {
            eager_end_to_end(
                black_box(rows.clone()),
                black_box(&stages),
                black_box(&settings),
            )
        });
        print_phase_case("stream first/all", iterations, || {
            streaming_end_to_end(
                black_box(rows.clone()),
                black_box(&stages),
                black_box(&settings),
            )
        });
    }
}

fn print_total_case(name: &str, iterations: u32, mut f: impl FnMut() -> WriteTiming) {
    for _ in 0..iterations.min(8) {
        black_box(f());
    }

    let start = Instant::now();
    for _ in 0..iterations {
        black_box(f());
    }
    let elapsed = start.elapsed();
    println!(
        "{name:20} total={:>8}  avg={:>10} us/op",
        format_duration(elapsed),
        micros_per_op(elapsed, iterations),
    );
}

fn print_phase_case(name: &str, iterations: u32, mut f: impl FnMut() -> WriteTiming) {
    for _ in 0..iterations.min(8) {
        black_box(f());
    }

    let mut first_sum = Duration::ZERO;
    let mut total_sum = Duration::ZERO;
    for _ in 0..iterations {
        let timing = black_box(f());
        first_sum += timing.first_write;
        total_sum += timing.total;
    }

    println!(
        "{name:20} first={:>10} us/op  all={:>10} us/op",
        micros_from_duration(first_sum, iterations),
        micros_from_duration(total_sum, iterations),
    );
}

fn render_and_write(output: &OutputResult, settings: &RenderSettings) -> WriteTiming {
    let start = Instant::now();
    let rendered = render_output(output, settings);
    let after_render = Instant::now();
    let mut writer = ProbeWriter::default();
    writer
        .write_all(rendered.as_bytes())
        .expect("probe write should succeed");
    writer.flush().expect("probe flush should succeed");

    WriteTiming {
        first_write: writer
            .first_write
            .unwrap_or(after_render)
            .saturating_duration_since(start),
        total: Instant::now().saturating_duration_since(start),
    }
}

fn eager_end_to_end(rows: Vec<Row>, stages: &[String], settings: &RenderSettings) -> WriteTiming {
    let start = Instant::now();
    let output = apply_pipeline(rows, stages).expect("eager passes");
    let execute_elapsed = start.elapsed();
    let render_timing = render_and_write(&output, settings);

    WriteTiming {
        first_write: execute_elapsed.saturating_add(render_timing.first_write),
        total: execute_elapsed.saturating_add(render_timing.total),
    }
}

fn streaming_end_to_end(
    rows: Vec<Row>,
    stages: &[String],
    settings: &RenderSettings,
) -> WriteTiming {
    let start = Instant::now();
    let output = execute_pipeline_streaming(rows, stages).expect("streaming passes");
    let execute_elapsed = start.elapsed();
    let render_timing = render_and_write(&output, settings);

    WriteTiming {
        first_write: execute_elapsed.saturating_add(render_timing.first_write),
        total: execute_elapsed.saturating_add(render_timing.total),
    }
}

fn micros_per_op(elapsed: Duration, iterations: u32) -> String {
    let per_op = elapsed.as_secs_f64() * 1_000_000.0 / f64::from(iterations.max(1));
    format!("{per_op:.2}")
}

fn micros_from_duration(duration: Duration, iterations: u32) -> String {
    let per_op = duration.as_secs_f64() * 1_000_000.0 / f64::from(iterations.max(1));
    format!("{per_op:.2}")
}

fn format_duration(duration: Duration) -> String {
    let millis = duration.as_secs_f64() * 1_000.0;
    format!("{millis:.2} ms")
}

#[derive(Debug, Clone, Copy, Default)]
struct WriteTiming {
    first_write: Duration,
    total: Duration,
}

#[derive(Default)]
struct ProbeWriter {
    first_write: Option<Instant>,
}

impl Write for ProbeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.first_write.is_none() && !buf.is_empty() {
            self.first_write = Some(Instant::now());
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn bench_rows(len: usize) -> Vec<Row> {
    (0..len)
        .map(|index| {
            json!({
                "uid": if index % 11 == 0 {
                    format!("alice-{index:05}")
                } else {
                    format!("user-{index:05}")
                },
                "mail": if index % 11 == 0 {
                    format!("alice{index}@example.org")
                } else {
                    format!("user{index}@example.org")
                },
                "groups": if index % 3 == 0 {
                    json!(["eng", "ops", format!("team-{index}")])
                } else {
                    json!(["sales", "support", format!("team-{index}")])
                },
                "profile": {
                    "department": if index % 2 == 0 { "engineering" } else { "operations" },
                    "enabled": index % 5 != 0,
                    "name": format!("Example User {index}")
                }
            })
            .as_object()
            .cloned()
            .expect("object")
        })
        .collect()
}
