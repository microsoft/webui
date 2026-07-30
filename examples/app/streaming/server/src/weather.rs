// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Delayed sample forecast endpoint for the independently hydrated weather
//! island.

use std::sync::Arc;

use actix_web::{web, HttpRequest, HttpResponse};
use serde_json::{Map, Value};

use crate::jitter::Jitter;
use crate::test_controls::TestSession;
use crate::{AppCtx, TEST_SESSION_COOKIE};

/// Simulated backend latency bounds for `GET /api/weather`.
pub(crate) const WEATHER_DELAY_MIN_MS: u64 = 700;
const WEATHER_DELAY_MAX_MS: u64 = 1_400;

const FORECASTS: [(&str, &str); 4] = [
    ("68°F", "Partly cloudy"),
    ("54°F", "Light rain"),
    ("72°F", "Clear"),
    ("61°F", "Overcast"),
];
const FORECAST_LOCATION: &str = "Redmond, WA";

/// The panel's own deliberately slow data source.
pub(crate) async fn weather_api(req: HttpRequest, ctx: web::Data<AppCtx>) -> HttpResponse {
    let mut jitter = Jitter::from_clock();
    if let Some(session) = test_session_from_cookie(&req, &ctx) {
        session.wait_for_weather().await;
    } else {
        let delay = jitter.delay_ms(WEATHER_DELAY_MIN_MS, WEATHER_DELAY_MAX_MS);
        tokio::time::sleep(delay).await;
    }

    let (temperature, condition) = FORECASTS[jitter.index(FORECASTS.len())];
    let mut forecast = Map::with_capacity(3);
    forecast.insert("location".to_owned(), Value::from(FORECAST_LOCATION));
    forecast.insert("temperature".to_owned(), Value::from(temperature));
    forecast.insert("condition".to_owned(), Value::from(condition));

    HttpResponse::Ok()
        .insert_header(("Cache-Control", "no-store"))
        .json(Value::Object(forecast))
}

fn test_session_from_cookie(req: &HttpRequest, ctx: &AppCtx) -> Option<Arc<TestSession>> {
    let id = req.cookie(TEST_SESSION_COOKIE)?;
    ctx.test_controls.as_ref()?.existing_session(id.value())
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
