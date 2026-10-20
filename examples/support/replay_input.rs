//! Example-only fixture loading. No networking or SDK runtime dependency.
use anyhow::{bail, ensure, Context, Result};
use serde_json::Value;

fn cases(text: &str) -> Result<Vec<Value>> {
    let mut document: Value = serde_json::from_str(text).context("invalid fixture JSON")?;
    let rows = if let Some(rows) = document.get_mut("cases") {
        let rows = rows.as_array_mut().context("fixture cases must be an array")?;
        ensure!(!rows.is_empty(), "fixture cases is empty");
        std::mem::take(rows)
    } else {
        vec![serde_json::json!({"rpc":document})]
    };
    rows.into_iter()
        .map(|mut case| {
            let object = case.as_object_mut().context("fixture case must be an object")?;
            let mut rpc = object
                .remove("wire_rpc")
                .or_else(|| object.remove("rpc"))
                .context("case needs rpc or wire_rpc")?;
            // getTransaction returns an envelope; captures may store just its result.
            if rpc.get("jsonrpc").is_some() {
                ensure!(rpc.get("error").is_none(), "RPC response contains an error");
                rpc = rpc.get_mut("result").context("RPC response needs result")?.take();
            }
            ensure!(
                rpc.get("slot").is_some() && rpc.get("transaction").is_some(),
                "expected saved RPC transaction (slot and transaction)"
            );
            object.insert("rpc".into(), rpc);
            Ok(case)
        })
        .collect()
}

/// None means --help was printed. Accept a raw saved RPC response or cases capture.
pub fn load(defaults: &[&str]) -> Result<Option<Vec<Value>>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [flag] if flag == "--help" || flag == "-h" => {
            println!("Usage: {} [--fixture FILE]\nDefault: bundled offline captures. No network or signing.\nFILE: saved RPC transaction or {{\"cases\":[{{\"rpc\":...}}]}} capture.", env!("CARGO_BIN_NAME"));
            Ok(None)
        }
        [] => Ok(Some(
            defaults
                .iter()
                .map(|text| cases(text))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect(),
        )),
        [flag, path] if flag == "--fixture" => {
            Ok(Some(cases(&std::fs::read_to_string(path).context("cannot read fixture")?)?))
        }
        _ => bail!("expected --fixture FILE or --help; no network fallback"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_raw_and_cases_but_rejects_empty_or_malformed_captures() {
        let rpc = r#"{"slot":1,"transaction":{}}"#;
        assert_eq!(cases(rpc).unwrap().len(), 1);
        assert_eq!(
            cases(&format!(r#"{{"jsonrpc":"2.0","result":{rpc},"id":1}}"#)).unwrap()[0]["rpc"]
                ["slot"],
            1
        );
        assert!(cases(r#"{"jsonrpc":"2.0","result":null,"id":1}"#).is_err());
        assert!(cases(r#"{"jsonrpc":"2.0","error":{"code":-1},"id":1}"#).is_err());
        assert_eq!(cases(&format!(r#"{{"cases":[{{"rpc":{rpc}}}]}}"#)).unwrap().len(), 1);
        let wire = cases(&format!(r#"{{"cases":[{{"wire_rpc":{rpc},"name":"wire"}}]}}"#)).unwrap();
        assert_eq!(wire[0]["rpc"]["slot"], 1);
        assert_eq!(wire[0]["name"], "wire");
        assert!(wire[0].get("wire_rpc").is_none());
        for text in [r#"{"cases":[]}"#, r#"{"cases":null}"#, r#"{"cases":[{}]}"#, "{}"] {
            assert!(cases(text).is_err());
        }
    }
}
