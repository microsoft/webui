// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Delayed server-side forecast producer for the updatable weather island.

use std::sync::Arc;

use serde_json::{Map, Value};

use crate::jitter::Jitter;
use crate::test_controls::TestSession;

/// Simulated backend latency bounds.
pub(crate) const WEATHER_DELAY_MIN_MS: u64 = 700;
const WEATHER_DELAY_MAX_MS: u64 = 1_400;

const FORECASTS: [(&str, &str); 4] = [
    ("68°F", "Partly cloudy"),
    ("54°F", "Light rain"),
    ("72°F", "Clear"),
    ("61°F", "Overcast"),
];
const FORECAST_LOCATION: &str = "Redmond, WA";

/// Resolve a forecast without creating a second browser request.
pub(crate) async fn load_forecast(test_session: Option<Arc<TestSession>>) -> Value {
    let mut jitter = Jitter::from_clock();
    if let Some(session) = test_session {
        session.wait_for_weather().await;
    } else {
        let delay = jitter.delay_ms(WEATHER_DELAY_MIN_MS, WEATHER_DELAY_MAX_MS);
        tokio::time::sleep(delay).await;
    }

    let (temperature, condition) = FORECASTS[jitter.index(FORECASTS.len())];
    let mut forecast = Map::with_capacity(4);
    forecast.insert("location".to_owned(), Value::from(FORECAST_LOCATION));
    forecast.insert("temperature".to_owned(), Value::from(temperature));
    forecast.insert("condition".to_owned(), Value::from(condition));
    forecast.insert("status".to_owned(), Value::from("ready"));
    Value::Object(forecast)
}

#[cfg(test)]
mod tests {
    use super::FORECASTS;

    #[test]
    fn every_forecast_sample_is_populated() {
        for (temperature, condition) in FORECASTS {
            assert!(!temperature.is_empty(), "a forecast has no temperature");
            assert!(!condition.is_empty(), "a forecast has no condition");
        }
    }
}
