mod commands;

use once_cell::sync::Lazy;
use anyhow::{bail, Result};
use twilight_interactions::command::CreateCommand;
use twilight_model::{
    application::{
        command::Command,
        interaction::{InteractionData, InteractionType, application_command::CommandData},
    },
    channel::message::MessageFlags,
    gateway::payload::incoming::InteractionCreate,
    http::interaction::{InteractionResponse, InteractionResponseType},
};
use twilight_interactions::command::{CommandModel};
use twilight_util::builder::InteractionResponseDataBuilder;
use std::{future::Future, sync::Arc};

use crate::StateRef;
use commands::{PlayCommand, LeaveCommand, JoinCommand};

use self::commands::SkipCommand;

#[allow(dead_code)]
pub static CREATE_GLOBAL_COMMANDS: Lazy<Vec<Command>> = Lazy::new(|| {
    vec![
        PlayCommand::create_command().into(),
        SkipCommand::create_command().into(),
        JoinCommand::create_command().into(),
        LeaveCommand::create_command().into(),
    ]
});

#[allow(dead_code)]
pub static CREATE_GUILD_COMMANDS: Lazy<Vec<Command>> = Lazy::new(|| {
    vec![
        
    ]
});

fn spawn<T>(
    fut: impl Future<Output = Result<T>> + Send + 'static,
) {
    tokio::spawn(async move {
        if let Err(why) = fut.await {
            tracing::debug!("handler error: {:?}", why);
        }
    });
}

pub async fn exec_command(state: Arc<StateRef>, cmd: &Box<CommandData>, interaction: Box<InteractionCreate>) -> Result<()> {
    match cmd.name.as_str() {
        "play" => {
            spawn(PlayCommand::from_interaction((**cmd).clone().into())?.run(state, interaction.0));
            Ok(())
        },
        "join" => {
            spawn(JoinCommand::from_interaction((**cmd).clone().into())?.run(state, interaction.0, false));
            Ok(())
        },
        "leave" => {
            spawn(LeaveCommand::from_interaction((**cmd).clone().into())?.run(state, interaction.0));
            Ok(())
        },
        "skip" => {
            spawn(SkipCommand::from_interaction((**cmd).clone().into())?.run(state, interaction.0));
            Ok(())
        }
        _ => bail!("Unknown command interaction {}", cmd.name),
    }
}

pub async fn handle_interaction(
    interaction: Box<InteractionCreate>,
    info: Arc<StateRef>,
) -> Result<()> {
    let interaction_clone = interaction.clone();
    if let Some(data) = &interaction_clone.0.data {
        match data {
            InteractionData::ApplicationCommand(cmd) => match interaction_clone.0.kind {
                InteractionType::ApplicationCommand => {
                    let command: Result<()> = exec_command(info.clone(), cmd, interaction).await;

                    if let Err(e) = &command {
                        let err_string = format!(
                            "An error occurred, it has been reported and will be fixed soon c:\n```\n{}\n```",
                            e
                        );
                        let client = info.http.interaction(info.application_id);
                        let msg_error = client
                            .create_response(
                                interaction_clone.0.id,
                                &interaction_clone.0.token,
                                &InteractionResponse {
                                    kind: InteractionResponseType::ChannelMessageWithSource,
                                    data: Some(
                                        InteractionResponseDataBuilder::new()
                                            .content(err_string.clone())
                                            .flags(MessageFlags::EPHEMERAL)
                                            .build(),
                                    ),
                                },
                            )
                            .await
                            .is_err();

                        //FIXME: maybe censor sensitive data?
                        if msg_error {
                            client
                                .update_response(&interaction_clone.0.token)
                                .attachments(&[])?
                                .components(Some(&[]))?
                                .embeds(Some(&[]))?
                                .content(Some(&err_string))?
                                .await?;
                        }
                    }

                    let channel = interaction_clone.0.channel.clone().unwrap();
                    println!(
                        "ID: {} <#{}> by <@{}>: /{} {:?}",
                        interaction_clone.0.id,
                        channel.id,
                        interaction_clone.0.author_id().unwrap(),
                        cmd.name,
                        cmd.options
                    );
                    command?
                }
                _ => {}
            },
            _ => {}
        }
    } else {
        match interaction.0.kind {
            _ => {}
        }
    }

    Ok(())
}