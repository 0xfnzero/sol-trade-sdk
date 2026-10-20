//! Strict CLI selection; parsing never opens a subscription.
use anyhow::{bail, ensure, Context, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    ParserGrpc,
    ParserShred,
    StreamerGrpc,
    StreamerShred,
}

pub fn parse(args: &[String]) -> Result<Option<(Source, bool)>> {
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        return Ok(None);
    }
    let mut source = None;
    let mut live = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--source" => {
                ensure!(source.is_none(), "--source may only be specified once");
                source = Some(match args.next().context("--source needs a value")?.as_str() {
                    "parser-grpc" => Source::ParserGrpc,
                    "parser-shred" => Source::ParserShred,
                    "streamer-grpc" => Source::StreamerGrpc,
                    "streamer-shred" => Source::StreamerShred,
                    _ => bail!(
                        "source must be parser-grpc, parser-shred, streamer-grpc or streamer-shred"
                    ),
                });
            }
            "--live" | "--replay" => {
                ensure!(live.is_none(), "choose exactly one of --live or --replay");
                live = Some(arg == "--live");
            }
            _ => bail!("unknown argument: {arg}; use --help"),
        }
    }
    Ok(Some((source.unwrap_or(Source::ParserGrpc), live.unwrap_or(false))))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_owned).collect()
    }
    #[test]
    fn offline_default_and_explicit_transport() {
        assert_eq!(parse(&[]).unwrap(), Some((Source::ParserGrpc, false)));
        assert_eq!(
            parse(&args("--source streamer-shred --live")).unwrap(),
            Some((Source::StreamerShred, true))
        );
        assert!(parse(&args("--help")).unwrap().is_none());
    }
    #[test]
    fn reject_ambiguous_modes_and_bad_sources_before_connecting() {
        for value in [
            "--live --replay",
            "--live --live",
            "--source",
            "--source other",
            "--source parser-grpc --source parser-shred",
            "--live --typo",
            "--help --live",
        ] {
            assert!(parse(&args(value)).is_err(), "{value}");
        }
    }
}
