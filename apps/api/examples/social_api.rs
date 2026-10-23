//! Executable documented requests; never prints credentials or response bodies.
use reqwest::{Client, Response, StatusCode};
use serde_json::{Value, json};
use std::error::Error;
use twitter_clone_api::social::{Page, Post};

pub struct Inputs {
    pub gateway: String,
    pub browser: String,
    pub write_key: String,
    pub read_key: String,
    pub read_key_id: String,
    pub session_cookie: String,
}

async fn expect(response: Response, status: StatusCode) -> Result<Response, Box<dyn Error>> {
    if response.status() != status {
        // Neither raw error bodies nor headers are safe diagnostics.
        return Err(format!(
            "Expected HTTP {status}, received HTTP {}",
            response.status()
        )
        .into());
    }
    Ok(response)
}

pub async fn run(input: Inputs) -> Result<(), Box<dyn Error>> {
    // Matches the management client's explicitly pinned TLS provider.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .no_proxy()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let gateway = input.gateway.trim_end_matches('/');
    let browser = input.browser.trim_end_matches('/');
    let mut created = Vec::new();
    for body in ["First API example post", "Second API example post"] {
        let response = client
            .post(format!("{gateway}/v1/posts"))
            .bearer_auth(&input.write_key)
            .json(&json!({"body":body}))
            .send()
            .await?;
        let post: Post = expect(response, StatusCode::CREATED).await?.json().await?;
        if post.body != body {
            return Err("Post response did not match the submitted text".into());
        }
        created.push(post.id);
    }
    let page: Page = expect(
        client
            .get(format!("{gateway}/v1/feed"))
            .bearer_auth(&input.read_key)
            .query(&[("limit", "1")])
            .send()
            .await?,
        StatusCode::OK,
    )
    .await?
    .json()
    .await?;
    let cursor = page
        .next_cursor
        .ok_or("Expected a cursor after creating two own posts")?;
    let next: Page = expect(
        client
            .get(format!("{gateway}/v1/feed"))
            .bearer_auth(&input.read_key)
            .query(&[("limit", "1"), ("cursor", cursor.as_str())])
            .send()
            .await?,
        StatusCode::OK,
    )
    .await?
    .json()
    .await?;
    if page.items.len() != 1 || next.items.len() != 1 || page.items[0].id == next.items[0].id {
        return Err("Expected distinct cursor-paginated posts".into());
    }
    let denied: Value = expect(
        client
            .post(format!("{gateway}/v1/posts"))
            .bearer_auth(&input.read_key)
            .json(&json!({"body":"Must not be created"}))
            .send()
            .await?,
        StatusCode::FORBIDDEN,
    )
    .await?
    .json()
    .await?;
    if denied["error"]["code"] != "permission_denied" {
        return Err("Expected the application's read-only permission denial".into());
    }
    // There is intentionally no integration key-management endpoint.
    expect(
        client
            .delete(format!("{gateway}/v1/auth/keys/{}", input.read_key_id))
            .bearer_auth(&input.write_key)
            .send()
            .await?,
        StatusCode::NOT_FOUND,
    )
    .await?;
    // Browser-only revocation: a verified session, original Origin, and a
    // bootstrap CSRF pair. No password, root credential, or key is a cookie.
    let bootstrap = expect(
        client
            .get(format!("{browser}/api/auth/csrf"))
            .header("Cookie", &input.session_cookie)
            .send()
            .await?,
        StatusCode::OK,
    )
    .await?;
    let mut cookies = input.session_cookie.clone();
    for value in bootstrap.headers().get_all(reqwest::header::SET_COOKIE) {
        let cookie = value
            .to_str()?
            .split(';')
            .next()
            .ok_or("Missing CSRF cookie")?;
        cookies.push_str("; ");
        cookies.push_str(cookie);
    }
    let bootstrap: Value = bootstrap.json().await?;
    expect(
        client
            .delete(format!("{browser}/api/auth/keys/{}", input.read_key_id))
            .header("Cookie", cookies)
            .header("Origin", browser)
            .header(
                "X-CSRF-Token",
                bootstrap["csrfToken"]
                    .as_str()
                    .ok_or("Missing CSRF token")?,
            )
            .send()
            .await?,
        StatusCode::NO_CONTENT,
    )
    .await?;
    // New requests fail even if the gateway still forwards cached identity.
    expect(
        client
            .get(format!("{gateway}/v1/me"))
            .bearer_auth(&input.read_key)
            .send()
            .await?,
        StatusCode::UNAUTHORIZED,
    )
    .await?;
    for id in created {
        expect(
            client
                .delete(format!("{gateway}/v1/posts/{id}"))
                .bearer_auth(&input.write_key)
                .send()
                .await?,
            StatusCode::NO_CONTENT,
        )
        .await?;
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let required =
        |name| std::env::var(name).map_err(|_| format!("Set {name} privately in the environment"));
    run(Inputs {
        gateway: required("API_URL")?,
        browser: required("WEB_URL")?,
        write_key: required("WRITE_KEY")?,
        read_key: required("READ_KEY")?,
        read_key_id: required("READ_KEY_ID")?,
        session_cookie: required("SESSION_COOKIE")?,
    })
    .await?;
    println!(
        "PASS: post creation, cursor paging, read-only denial, browser-only revocation, and cleanup"
    );
    Ok(())
}
