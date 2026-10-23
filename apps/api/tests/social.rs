use sqlx::PgPool;
use twitter_clone_api::{
    accounts::Accounts,
    database::MIGRATOR,
    social::{Actor, PageRequest, PostCreate, ProfileUpdate, Social, SocialError},
};
use uuid::Uuid;

mod support;
const PASSWORD: &str = "disposable social fixture password";

async fn setup() -> (PgPool, Accounts, Social) {
    let pool = support::fresh_pool().await;
    MIGRATOR.run(&pool).await.unwrap();
    let accounts = Accounts::new(pool.clone()).await.unwrap();
    let social = Social::new(
        pool.clone(),
        &std::env::var("RATE_LIMIT_SECRET").unwrap(),
        30,
    )
    .unwrap();
    (pool, accounts, social)
}

async fn actor(pool: &PgPool, accounts: &Accounts, handle: &str, verified: bool) -> Actor {
    let email = format!("{handle}@example.test");
    accounts.register(&email, handle, PASSWORD).await.unwrap();
    if verified {
        sqlx::query("UPDATE users SET verified_at = now() WHERE handle = $1")
            .bind(handle)
            .execute(pool)
            .await
            .unwrap();
    }
    let grant = accounts.login(&email, PASSWORD, None).await.unwrap();
    let session = accounts
        .resolve_session(grant.token.expose_secret())
        .await
        .unwrap()
        .unwrap();
    Actor::from_session(&session)
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn profiles_are_public_private_fields_never_serialize_and_counts_are_current() {
    let (pool, accounts, social) = setup().await;
    let alice = actor(&pool, &accounts, "Alice", true).await;
    let bob = actor(&pool, &accounts, "Bob", true).await;
    sqlx::query("INSERT INTO follows (follower_id, followee_id) VALUES ($1, $2)")
        .bind(alice.id())
        .bind(bob.id())
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO posts (id, author_id, body) VALUES ($1, $2, 'fixture')")
        .bind(Uuid::new_v4())
        .bind(bob.id())
        .execute(&pool)
        .await
        .unwrap();
    let profile = social.profile("BOB", None).await.unwrap();
    assert_eq!(profile.handle, "Bob");
    assert_eq!(profile.display_name, "Bob");
    assert_eq!(
        (
            profile.follower_count,
            profile.following_count,
            profile.post_count
        ),
        (1, 0, 1)
    );
    assert!(!profile.viewer_follows);
    assert!(
        social
            .profile("Bob", Some(alice))
            .await
            .unwrap()
            .viewer_follows
    );
    let value = serde_json::to_value(profile).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 9);
    for private in [
        "email",
        "password",
        "verified",
        "token",
        "key",
        "digest",
        "@example.test",
        "$argon2",
    ] {
        assert!(!value.to_string().contains(private));
    }
    assert!(matches!(
        social.profile("absent", None).await,
        Err(SocialError::NotFound)
    ));
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn profile_edits_require_verified_owner_preserve_handles_and_count_code_points() {
    let (pool, accounts, social) = setup().await;
    let alice = actor(&pool, &accounts, "Alice", true).await;
    let bob = actor(&pool, &accounts, "Bob", true).await;
    let unverified = actor(&pool, &accounts, "Pending", false).await;
    let update = || ProfileUpdate {
        display_name: "🦀".repeat(50),
        biography: "é".repeat(160),
    };
    assert!(matches!(
        social.update_profile(alice, bob.id(), update()).await,
        Err(SocialError::Forbidden)
    ));
    assert!(matches!(
        social
            .update_profile(unverified, unverified.id(), update())
            .await,
        Err(SocialError::VerificationRequired)
    ));
    let edited = social
        .update_profile(alice, alice.id(), update())
        .await
        .unwrap();
    assert_eq!(edited.handle, "Alice");
    assert_eq!(edited.display_name.chars().count(), 50);
    assert_eq!(edited.biography.chars().count(), 160);
    for (display_name, biography) in [
        (String::new(), "".into()),
        ("🦀".repeat(51), "".into()),
        ("Alice".into(), "é".repeat(161)),
    ] {
        assert!(matches!(
            social
                .update_profile(
                    alice,
                    alice.id(),
                    ProfileUpdate {
                        display_name,
                        biography
                    }
                )
                .await,
            Err(SocialError::InvalidProfile)
        ));
    }
    let minimum = social
        .update_profile(
            alice,
            alice.id(),
            ProfileUpdate {
                display_name: "x".into(),
                biography: "".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(minimum.display_name, "x");
    assert!(minimum.biography.is_empty());
    assert!(
        serde_json::from_value::<ProfileUpdate>(
            serde_json::json!({"displayName":"spoof", "biography":"", "handle":"changed"})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<ProfileUpdate>(
            serde_json::json!({"displayName":"spoof", "biography":"", "userId":bob.id()})
        )
        .is_err()
    );
    assert_eq!(social.me(bob).await.unwrap().display_name, "Bob");
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn posts_count_unicode_code_points_preserve_text_and_never_accept_an_author_from_input() {
    let (pool, accounts, social) = setup().await;
    let author = actor(&pool, &accounts, "Author", true).await;
    let pending = actor(&pool, &accounts, "Pending", false).await;
    for body in [
        "",
        " ",
        "\r\n\t",
        "\u{a0}\u{2003}",
        &"x".repeat(281),
        &"🦀".repeat(281),
    ] {
        assert!(matches!(
            social
                .create_post(author, PostCreate { body: body.into() })
                .await,
            Err(SocialError::InvalidPost)
        ));
    }
    for body in [
        "x",
        &"x".repeat(280),
        &"🦀".repeat(280),
        " <script>alert('plain text')</script> \n",
    ] {
        let created = social
            .create_post(author, PostCreate { body: body.into() })
            .await
            .unwrap();
        assert_eq!(created.author_id, author.id());
        assert_eq!(created.author_handle, "Author");
        assert_eq!(created.body, body);
        let read = social.post(created.id, None).await.unwrap();
        assert_eq!(read.body, body);
        assert_eq!(read.like_count, 0);
        assert!(!read.viewer_likes);
        let json = serde_json::to_value(read).unwrap();
        assert_eq!(json.as_object().unwrap().len(), 8);
        for private in ["email", "password", "token", "digest"] {
            assert!(!json.to_string().contains(private));
        }
    }
    assert!(matches!(
        social
            .create_post(pending, PostCreate { body: "no".into() })
            .await,
        Err(SocialError::VerificationRequired)
    ));
    assert!(
        serde_json::from_value::<PostCreate>(
            serde_json::json!({"body":"spoof", "authorId":pending.id()})
        )
        .is_err()
    );
    assert_eq!(social.me(author).await.unwrap().post_count, 4);
    assert_eq!(social.me(pending).await.unwrap().post_count, 0);
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn only_verified_author_can_delete_and_deletion_cascades_likes_and_counts() {
    let (pool, accounts, social) = setup().await;
    let author = actor(&pool, &accounts, "Author", true).await;
    let other = actor(&pool, &accounts, "Other", true).await;
    let pending = actor(&pool, &accounts, "Pending", false).await;
    let post = social
        .create_post(
            author,
            PostCreate {
                body: "fixture".into(),
            },
        )
        .await
        .unwrap();
    sqlx::query("INSERT INTO likes (user_id, post_id) VALUES ($1, $2)")
        .bind(other.id())
        .bind(post.id)
        .execute(&pool)
        .await
        .unwrap();
    let read = social.post(post.id, Some(other)).await.unwrap();
    assert_eq!(read.like_count, 1);
    assert!(read.viewer_likes);
    assert!(matches!(
        social.delete_post(other, post.id).await,
        Err(SocialError::Forbidden)
    ));
    assert!(matches!(
        social.delete_post(pending, post.id).await,
        Err(SocialError::VerificationRequired)
    ));
    assert!(social.post(post.id, None).await.is_ok());
    social.delete_post(author, post.id).await.unwrap();
    assert!(matches!(
        social.post(post.id, None).await,
        Err(SocialError::NotFound)
    ));
    assert!(matches!(
        social.delete_post(author, post.id).await,
        Err(SocialError::NotFound)
    ));
    assert!(matches!(
        social.post(Uuid::new_v4(), None).await,
        Err(SocialError::NotFound)
    ));
    assert_eq!(social.me(author).await.unwrap().post_count, 0);
    let likes: i64 = sqlx::query_scalar("SELECT count(*) FROM likes")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(likes, 0);
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn concurrent_repeated_relationship_changes_are_idempotent_and_counts_follow_state() {
    let (pool, accounts, _) = setup().await;
    let social = Social::new(
        pool.clone(),
        &std::env::var("RATE_LIMIT_SECRET").unwrap(),
        200,
    )
    .unwrap();
    let alice = actor(&pool, &accounts, "Alice", true).await;
    let bob = actor(&pool, &accounts, "Bob", true).await;
    let post = social
        .create_post(
            bob,
            PostCreate {
                body: "fixture".into(),
            },
        )
        .await
        .unwrap();
    for following in [true, false] {
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..12 {
            let social = social.clone();
            tasks.spawn(async move { social.set_following(alice, bob.id(), following).await });
        }
        while let Some(result) = tasks.join_next().await {
            result.unwrap().unwrap();
        }
        assert_eq!(
            social.me(alice).await.unwrap().following_count,
            i64::from(following)
        );
        assert_eq!(
            social.me(bob).await.unwrap().follower_count,
            i64::from(following)
        );
        assert_eq!(
            social
                .profile("Bob", Some(alice))
                .await
                .unwrap()
                .viewer_follows,
            following
        );
    }
    for liked in [true, false] {
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..12 {
            let social = social.clone();
            tasks.spawn(async move { social.set_like(alice, post.id, liked).await });
        }
        while let Some(result) = tasks.join_next().await {
            result.unwrap().unwrap();
        }
        let read = social.post(post.id, Some(alice)).await.unwrap();
        assert_eq!(read.like_count, i64::from(liked));
        assert_eq!(read.viewer_likes, liked);
    }
    assert!(matches!(
        social.set_following(alice, alice.id(), true).await,
        Err(SocialError::SelfFollow)
    ));
    for present in [true, false] {
        assert!(matches!(
            social.set_following(alice, Uuid::new_v4(), present).await,
            Err(SocialError::NotFound)
        ));
        assert!(matches!(
            social.set_like(alice, Uuid::new_v4(), present).await,
            Err(SocialError::NotFound)
        ));
    }
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn independent_social_instances_share_one_user_budget_across_operation_types() {
    use twitter_clone_api::rate_limit::RateLimitError;
    let (pool, accounts, _) = setup().await;
    let secret = std::env::var("RATE_LIMIT_SECRET").unwrap();
    let first = Social::new(pool.clone(), &secret, 3).unwrap();
    let second = Social::new(pool.clone(), &secret, 3).unwrap();
    let alice = actor(&pool, &accounts, "Alice", true).await;
    let bob = actor(&pool, &accounts, "Bob", true).await;
    let post = first
        .create_post(
            bob,
            PostCreate {
                body: "fixture".into(),
            },
        )
        .await
        .unwrap();
    first.set_following(alice, bob.id(), true).await.unwrap();
    second.set_like(alice, post.id, true).await.unwrap();
    first
        .update_profile(
            alice,
            alice.id(),
            ProfileUpdate {
                display_name: "Alice".into(),
                biography: "fixture".into(),
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        second
            .create_post(
                alice,
                PostCreate {
                    body: "limited".into()
                }
            )
            .await,
        Err(SocialError::RateLimit(RateLimitError::Limited { .. }))
    ));
    assert_eq!(first.me(alice).await.unwrap().post_count, 0);
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn timeline_cursors_handle_timestamp_ties_insertions_deletions_and_current_follows() {
    let (pool, accounts, social) = setup().await;
    let alice = actor(&pool, &accounts, "Alice", true).await;
    let bob = actor(&pool, &accounts, "Bob", true).await;
    let stranger = actor(&pool, &accounts, "Stranger", true).await;
    let pending = actor(&pool, &accounts, "Pending", false).await;
    assert!(
        social
            .feed(alice, PageRequest::default())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert!(
        social
            .profile_posts("Alice", None, PageRequest::default())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert!(matches!(
        social.feed(pending, PageRequest::default()).await,
        Err(SocialError::VerificationRequired)
    ));
    social.set_following(alice, bob.id(), true).await.unwrap();
    for (id, author) in [
        (1, alice),
        (2, bob),
        (3, stranger),
        (4, alice),
        (5, bob),
        (6, alice),
    ] {
        sqlx::query("INSERT INTO posts (id, author_id, body, created_at) VALUES ($1, $2, 'fixture', '2026-01-01T00:00:00Z')")
            .bind(Uuid::from_u128(id)).bind(author.id()).execute(&pool).await.unwrap();
    }
    let first = social
        .feed(
            alice,
            PageRequest {
                limit: Some("2".into()),
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        first
            .items
            .iter()
            .map(|p| p.id.as_u128())
            .collect::<Vec<_>>(),
        vec![6, 5]
    );
    let cursor = first.next_cursor.unwrap();
    // This new post sorts ahead of the cursor; it must not repeat old results.
    social
        .create_post(
            bob,
            PostCreate {
                body: "insert between pages".into(),
            },
        )
        .await
        .unwrap();
    // The cursor's anchor can disappear without invalidating the next page.
    social.delete_post(bob, Uuid::from_u128(5)).await.unwrap();
    let second = social
        .feed(
            alice,
            PageRequest {
                limit: Some("2".into()),
                cursor: Some(cursor.clone()),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        second
            .items
            .iter()
            .map(|p| p.id.as_u128())
            .collect::<Vec<_>>(),
        vec![4, 2]
    );
    let third = social
        .feed(
            alice,
            PageRequest {
                limit: Some("2".into()),
                cursor: second.next_cursor,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        third
            .items
            .iter()
            .map(|p| p.id.as_u128())
            .collect::<Vec<_>>(),
        vec![1]
    );
    assert!(third.next_cursor.is_none());
    social.set_following(alice, bob.id(), false).await.unwrap();
    let after_unfollow = social
        .feed(
            alice,
            PageRequest {
                limit: Some("100".into()),
                cursor: Some(cursor),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        after_unfollow
            .items
            .iter()
            .map(|p| p.id.as_u128())
            .collect::<Vec<_>>(),
        vec![4, 1]
    );
    let public = social
        .profile_posts("ALICE", None, PageRequest::default())
        .await
        .unwrap();
    assert_eq!(
        public
            .items
            .iter()
            .map(|p| p.id.as_u128())
            .collect::<Vec<_>>(),
        vec![6, 4, 1]
    );
    assert!(matches!(
        social
            .profile_posts("absent", None, PageRequest::default())
            .await,
        Err(SocialError::NotFound)
    ));
    for page in [
        PageRequest {
            limit: Some("101".into()),
            cursor: None,
        },
        PageRequest {
            limit: None,
            cursor: Some("invalid".into()),
        },
    ] {
        assert!(matches!(
            social.feed(alice, page).await,
            Err(SocialError::InvalidPagination)
        ));
    }
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL; run through tests/with_postgres.py"]
async fn timelines_default_to_twenty_cap_at_one_hundred_and_finish_without_repeats() {
    let (pool, accounts, social) = setup().await;
    let alice = actor(&pool, &accounts, "Alice", true).await;
    sqlx::query("INSERT INTO posts (id, author_id, body) SELECT gen_random_uuid(), $1, 'fixture' FROM generate_series(1,105)")
        .bind(alice.id()).execute(&pool).await.unwrap();
    let default_page = social.feed(alice, PageRequest::default()).await.unwrap();
    assert_eq!(default_page.items.len(), 20);
    assert!(default_page.next_cursor.is_some());
    let first = social
        .profile_posts(
            "Alice",
            None,
            PageRequest {
                limit: Some("100".into()),
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(first.items.len(), 100);
    let second = social
        .profile_posts(
            "Alice",
            None,
            PageRequest {
                limit: Some("100".into()),
                cursor: first.next_cursor,
            },
        )
        .await
        .unwrap();
    assert_eq!(second.items.len(), 5);
    assert!(second.next_cursor.is_none());
    let unique: std::collections::HashSet<_> = first
        .items
        .iter()
        .chain(&second.items)
        .map(|p| p.id)
        .collect();
    assert_eq!(unique.len(), 105);
    pool.close().await;
}
