//! Thin AT Protocol wrapper: log in with an App Password and create a feed post
//! carrying an `app.bsky.embed.external` card.

use anyhow::{Context, Result};
use bsky_sdk::agent::config::Config as AgentConfig;
use bsky_sdk::api::app::bsky::embed::external::{ExternalData, MainData};
use bsky_sdk::api::app::bsky::feed::post::{RecordData, RecordEmbedRefs};
use bsky_sdk::api::types::string::Datetime;
use bsky_sdk::api::types::Union;
use bsky_sdk::BskyAgent;

use crate::format::ComposedPost;

pub struct BlueskyClient {
    agent: BskyAgent,
}

impl BlueskyClient {
    pub async fn login(service: &str, handle: &str, app_password: &str) -> Result<Self> {
        let agent = BskyAgent::builder()
            .config(AgentConfig {
                endpoint: service.to_string(),
                ..Default::default()
            })
            .build()
            .await
            .context("failed to build Bluesky agent")?;
        agent
            .login(handle, app_password)
            .await
            .context("Bluesky login failed")?;
        Ok(Self { agent })
    }

    /// Create a post and return its AT URI.
    pub async fn post(&self, post: &ComposedPost) -> Result<String> {
        let main = MainData {
            external: ExternalData {
                uri: post.embed.uri.clone(),
                title: post.embed.title.clone(),
                description: post.embed.description.clone(),
                thumb: None,
            }
            .into(),
        };
        let embed = Union::Refs(RecordEmbedRefs::AppBskyEmbedExternalMain(Box::new(
            main.into(),
        )));

        let record = RecordData {
            created_at: Datetime::now(),
            embed: Some(embed),
            entities: None,
            facets: None,
            labels: None,
            langs: None,
            reply: None,
            tags: None,
            text: post.text.clone(),
        };

        let output = self
            .agent
            .create_record(record)
            .await
            .context("failed to create Bluesky post")?;
        Ok(output.uri.clone())
    }
}
