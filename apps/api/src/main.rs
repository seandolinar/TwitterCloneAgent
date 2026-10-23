use std::{
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream},
    sync::Arc,
    time::Duration,
};
use twitter_clone_api::{
    accounts::EmailDelivery, browser::BrowserAuth, config::Config, database, http, maintenance,
    social::Social,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Arc::new(Config::from_env()?);
    let command = std::env::args().nth(1);
    match command.as_deref() {
        Some("healthcheck") => return healthcheck(config.browser_bind),
        Some("migrate") | None => {}
        Some(_) => return Err("Usage: twitter-clone-api [healthcheck|migrate]".into()),
    }
    let pool = database::connect(&config.database_url)
        .await
        .map_err(|_| "Database connection failed; check DATABASE_URL and database availability")?;
    if command.as_deref() == Some("migrate") {
        database::MIGRATOR.run(&pool).await.map_err(
            |_| "Database migration failed; inspect schema compatibility before retrying",
        )?;
        pool.close().await;
        return Ok(());
    }
    if !database::is_ready(&pool).await {
        return Err("Database schema is not ready; run the migrate command first".into());
    }
    let (browser, gateway) = tokio::try_join!(
        tokio::net::TcpListener::bind(config.browser_bind),
        tokio::net::TcpListener::bind(config.gateway_bind),
    )?;
    eprintln!("Browser API listening on {}", config.browser_bind);
    eprintln!("Gateway API listening on {}", config.gateway_bind);
    let auth = Arc::new(BrowserAuth::new(pool.clone(), config.clone()).await?);
    let social = Social::new(
        pool.clone(),
        &config.rate_limit_secret,
        config.social_writes_per_minute,
    )
    .map_err(|_| "Social service configuration is invalid")?;
    let maintenance = tokio::spawn(maintenance::run(pool.clone()));
    let delivery = tokio::spawn(EmailDelivery::new(pool.clone(), &config)?.run());
    let cleanup = tokio::spawn(auth.keys.clone().run());
    let servers = tokio::try_join!(
        axum::serve(
            browser,
            http::browser_router(pool.clone(), &auth, social.clone())
                .into_make_service_with_connect_info::<SocketAddr>()
        )
        .with_graceful_shutdown(shutdown()),
        axum::serve(
            gateway,
            http::gateway_router(
                pool.clone(),
                social,
                config.unkey.as_ref().map(|unkey| unkey.keyspace_id.clone())
            )
        )
        .with_graceful_shutdown(shutdown()),
    );
    maintenance.abort();
    delivery.abort();
    cleanup.abort();
    let _ = maintenance.await;
    let _ = delivery.await;
    let _ = cleanup.await;
    pool.close().await;
    servers?;
    Ok(())
}

fn healthcheck(mut address: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
    if address.ip().is_unspecified() {
        address.set_ip(if address.is_ipv4() {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        } else {
            IpAddr::V6(Ipv6Addr::LOCALHOST)
        });
    }
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(b"GET /readyz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
    let mut response = String::new();
    stream.take(4096).read_to_string(&mut response)?;
    if !response.starts_with("HTTP/1.1 200 ") && !response.starts_with("HTTP/1.0 200 ") {
        return Err("Health check failed".into());
    }
    Ok(())
}

async fn shutdown() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("SIGTERM handler must be available");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}
