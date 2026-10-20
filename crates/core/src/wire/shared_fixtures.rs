//! Runs the shared wire fixtures (`packages/wire/fixtures`, ARCHITECTURE §1.7): every
//! `aasp/*.txt` stream against its `*.json` expectation (parsed whole and one byte at a time), and
//! every `worker/*.json` value through its serde type (must round-trip unchanged). The TypeScript
//! AASP reader runs the same `aasp/` files in `packages/ai/test/aasp-fixtures.test.ts`.

use std::path::PathBuf;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::aasp::{AaspEvent, AaspParser};
use super::worker;

fn dir(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packages/wire/fixtures").join(name)
}

fn files(name: &str, ext: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir(name))
        .unwrap_or_else(|e| panic!("packages/wire/fixtures/{name}: {e}"))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == ext))
        .collect();
    out.sort();
    out
}

fn as_json(e: &AaspEvent) -> Value {
    serde_json::json!({"event": e.name(), "data": serde_json::from_str::<Value>(&e.data()).unwrap()})
}

fn run(chunks: &[&[u8]]) -> (Vec<Value>, usize) {
    let mut p = AaspParser::new();
    let mut results = Vec::new();
    for c in chunks {
        results.extend(p.feed(c));
    }
    results.extend(p.finish());
    let rejected = results.iter().filter(|r| r.is_err()).count();
    (results.iter().filter_map(|r| r.as_ref().ok()).map(as_json).collect(), rejected)
}

#[test]
fn aasp_streams() {
    let txts = files("aasp", "txt");
    assert!(txts.len() >= 10, "aasp fixtures missing");
    for txt in txts {
        let name = txt.display().to_string();
        let input = std::fs::read(&txt).unwrap();
        let expected: Value =
            serde_json::from_str(&std::fs::read_to_string(txt.with_extension("json")).unwrap()).unwrap();
        let want = (expected["events"].as_array().unwrap().clone(), expected["rejected"].as_u64().unwrap() as usize);
        assert_eq!(run(&[&input]), want, "{name} (whole)");
        let bytes: Vec<&[u8]> = input.chunks(1).collect();
        assert_eq!(run(&bytes), want, "{name} (byte by byte)");
        // Every valid event re-encodes to a frame that parses back to itself.
        for e in &want.0 {
            let mut p = AaspParser::new();
            let ev = AaspEvent::decode(e["event"].as_str().unwrap(), &e["data"].to_string()).unwrap();
            assert_eq!(p.feed(ev.encode().as_bytes()), vec![Ok(ev)], "{name}");
        }
    }
}

fn round_trip<T: Serialize + DeserializeOwned>(value: &Value) -> Value {
    let typed: T = serde_json::from_value(value.clone()).unwrap_or_else(|e| panic!("{e}: {value}"));
    serde_json::to_value(typed).unwrap()
}

#[test]
fn worker_values_round_trip() {
    let jsons = files("worker", "json");
    assert!(jsons.len() >= 10, "worker fixtures missing");
    for path in jsons {
        let name = path.display().to_string();
        let fx: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let value = &fx["value"];
        let back = match fx["type"].as_str().unwrap() {
            "ConnectStart" => round_trip::<worker::ConnectStart>(value),
            "ConnectComplete" => round_trip::<worker::ConnectComplete>(value),
            "ConnectCompleteResponse" => round_trip::<worker::ConnectCompleteResponse>(value),
            "ImapDiscovery" => round_trip::<worker::ImapDiscovery>(value),
            "ImapConnect" => round_trip::<worker::ImapConnect>(value),
            "MessageBody" => round_trip::<worker::MessageBody>(value),
            "DraftPut" => round_trip::<worker::DraftPut>(value),
            "DraftSaved" => round_trip::<worker::DraftSaved>(value),
            "SendRequest" => round_trip::<worker::SendRequest>(value),
            "SendAccepted" => round_trip::<worker::SendAccepted>(value),
            other => panic!("{name}: no Rust type for {other}"),
        };
        assert_eq!(&back, value, "{name}");
    }
}
