use std::io;

use crate::io_other;
use crate::model::{BenchmarkConfig, ParseOutcome};

pub(crate) fn parse_args() -> io::Result<BenchmarkConfig> {
    let mut config = BenchmarkConfig::default();
    let outcome = parse_args_from(std::env::args().skip(1), &mut config)?;
    if outcome == ParseOutcome::HelpRequested {
        print_help();
        std::process::exit(0);
    }
    Ok(config)
}

pub(crate) fn parse_args_from(
    args: impl IntoIterator<Item = String>,
    config: &mut BenchmarkConfig,
) -> io::Result<ParseOutcome> {
    let mut args = args.into_iter();

    while let Some(flag) = args.next() {
        match flag.as_str() {
            "-h" | "--help" => return Ok(ParseOutcome::HelpRequested),
            "--profile-name" => config.profile_name = next_value(&mut args, "--profile-name")?,
            "--host" => config.host = next_value(&mut args, "--host")?,
            "--port" => {
                config.port = next_value(&mut args, "--port")?
                    .parse::<u16>()
                    .map_err(|error| io_other(format!("invalid --port value: {error}")))?;
            }
            "--user" => config.user = next_value(&mut args, "--user")?,
            "--database" => config.database = next_value(&mut args, "--database")?,
            "--sql" => config.sql = next_value(&mut args, "--sql")?,
            "--seed-rows" => {
                config.seed_rows = next_value(&mut args, "--seed-rows")?
                    .parse::<u64>()
                    .map_err(|error| io_other(format!("invalid --seed-rows value: {error}")))?;
            }
            "--assert-first-row-ms" => {
                config.assert_first_row_ms = Some(
                    next_value(&mut args, "--assert-first-row-ms")?
                        .parse::<f64>()
                        .map_err(|error| {
                            io_other(format!("invalid --assert-first-row-ms value: {error}"))
                        })?,
                );
            }
            "--assert-min-rows-per-sec" => {
                config.assert_min_rows_per_sec = Some(
                    next_value(&mut args, "--assert-min-rows-per-sec")?
                        .parse::<f64>()
                        .map_err(|error| {
                            io_other(format!("invalid --assert-min-rows-per-sec value: {error}"))
                        })?,
                );
            }
            "--trend-policy" => {
                config.trend_policy = Some(next_value(&mut args, "--trend-policy")?);
            }
            "--metrics-output" => {
                config.metrics_output = Some(next_value(&mut args, "--metrics-output")?);
            }
            "--metrics-label" => {
                config.metrics_label = Some(next_value(&mut args, "--metrics-label")?);
            }
            _ => {
                return Err(io_other(format!("unknown argument `{flag}`")));
            }
        }
    }

    Ok(ParseOutcome::Config)
}

pub(crate) fn next_value(
    args: &mut impl Iterator<Item = String>,
    flag: &str,
) -> io::Result<String> {
    args.next()
        .ok_or_else(|| io_other(format!("missing value for `{flag}`")))
}

fn print_help() {
    println!(
        "myr benchmark runner\n\n\
Usage:\n  cargo run -p myr-app --bin benchmark -- [OPTIONS]\n\n\
Options:\n  --profile-name <name>           Profile name used for connection manager (default: bench-local)\n  --host <host>                   MySQL host (default: 127.0.0.1)\n  --port <port>                   MySQL port (default: 3306)\n  --user <user>                   MySQL user (default: root)\n  --database <name>               Database name (default: myr_bench)\n  --sql <query>                   Query to benchmark\n  --seed-rows <count>             Seed `events` table up to count rows before benchmark\n  --assert-first-row-ms <ms>      Fail if first-row latency exceeds threshold\n  --assert-min-rows-per-sec <rps> Fail if throughput is below threshold\n  --trend-policy <path>           Enforce trend policy from baseline/tolerance JSON\n  --metrics-output <path>         Write machine-readable benchmark JSON\n  --metrics-label <label>         Optional label stored in metrics output\n\n\
Environment:\n  MYR_DB_PASSWORD is used for authentication.\n"
    );
}
