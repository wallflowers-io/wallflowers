//! Media is never deleted (#56, ruled 9 Nov 2026), so the budget fills. At 80% the relay
//! says so at ERROR, which reaches Sentry, once an hour rather than on every reading, and
//! `GET /health` carries the share without turning the relay unhealthy.

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use relay::health::Health;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Captured {
    fn lines(&self, level: &str) -> Vec<String> {
        String::from_utf8(self.0.lock().unwrap().clone())
            .unwrap()
            .lines()
            .filter(|l| l.contains(level))
            .map(str::to_string)
            .collect()
    }
}

const BUDGET: u64 = 9_000_000_000;

#[test]
fn above_eighty_percent_is_said_once_an_hour_and_health_stays_ok() {
    let log = Captured::default();
    let w = log.clone();
    let _g = tracing::subscriber::set_default(
        tracing_subscriber::fmt().with_ansi(false).with_writer(move || w.clone()).finish(),
    );
    let h = Health::new(15);
    h.set(true, Some(50));
    let mut said = None;
    let t0 = Instant::now();

    assert_eq!(h.media_used_pct(), None, "unknown until the meter has read the bucket");
    assert_eq!(h.media_measured(BUDGET * 85 / 100, BUDGET, &mut said, t0), 85);
    h.media_measured(BUDGET * 86 / 100, BUDGET, &mut said, t0 + Duration::from_secs(900));
    assert_eq!(log.lines("ERROR").len(), 1, "two readings within the hour, one alarm");

    let body: serde_json::Value = serde_json::from_str(&h.body()).unwrap();
    assert_eq!(body["media_used_pct"], 86);
    assert!(h.ok(), "80% is a warning: uploads still work, so the relay is not unhealthy");

    h.media_measured(BUDGET * 86 / 100, BUDGET, &mut said, t0 + Duration::from_secs(3600));
    assert_eq!(log.lines("ERROR").len(), 2, "said again after an hour");

    h.media_measured(BUDGET / 2, BUDGET, &mut said, t0 + Duration::from_secs(4000));
    h.media_measured(BUDGET / 2, BUDGET, &mut said, t0 + Duration::from_secs(4900));
    let back: Vec<_> = log.lines(" INFO").into_iter().filter(|l| l.contains("back below")).collect();
    assert_eq!(back.len(), 1, "recovery said once");
    assert_eq!(log.lines("ERROR").len(), 2);
}
