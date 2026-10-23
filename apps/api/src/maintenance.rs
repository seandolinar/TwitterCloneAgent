use sqlx::PgPool;
use std::time::Duration;

pub async fn run(pool: PgPool) {
    let mut ticks = tokio::time::interval(Duration::from_secs(60));
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        ticks.tick().await;
        if crate::rate_limit::cleanup_expired(&pool, 1000)
            .await
            .is_err()
        {
            eprintln!("Rate-limit cleanup temporarily unavailable");
        }
        if crate::accounts::cleanup_expired_security_records(&pool, 1000)
            .await
            .is_err()
        {
            eprintln!("Session/token cleanup temporarily unavailable");
        }
    }
}
