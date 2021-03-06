use diesel::{r2d2::{PooledConnection, ConnectionManager}, PgConnection};
use itertools::Itertools;
use rand::{seq::SliceRandom, thread_rng};
use serde_json;
use serenity;
use serenity::{
    builder::CreateMessage,
    framework::standard::{Args, CommandOptions},
    model::{
        channel::Message,
        guild::Member,
        id::{ChannelId, GuildId, MessageId, UserId},
    },
    prelude::*,
    utils::with_cache,
};
use std::fmt::Display;

#[macro_use]
pub mod macros;
pub mod markov;
pub mod pagination;

pub fn names_for_members<U, G>(u_ids: &[U], g_id: G) -> Vec<String>
where
    U: Into<UserId> + Copy,
    G: Into<GuildId> + Copy,
{
    fn backup_getter(u_id: impl Into<UserId> + Copy) -> String {
        match u_id.into().to_user() {
            Ok(u) => u.name,
            _ => u_id.into().to_string(),
        }
    }

    log_time!(
        with_cache(|cache| cache.guild(g_id).map(|g| {
            let members = &g.read().members;
            u_ids
                .iter()
                .map(|&id| {
                    members
                        .get(&id.into())
                        .map_or_else(|| backup_getter(id), |m| m.display_name().to_string())
                })
                .collect()
        }))
        .unwrap_or_else(|| u_ids.iter().map(|&id| backup_getter(id)).collect()),
        "with_cache: find_members"
    )
}

pub fn and_comma_split<T: AsRef<str>>(m: &[T]) -> String {
    let len = m.len();

    match m {
        [] => "".to_owned(),
        [a] => a.as_ref().to_owned(),
        _ => {
            let mut res = String::new();
            let mut iter = m.into_iter();
            res.push_str(&iter.take(len - 1).map(|s| s.as_ref()).join(", "));
            res.push_str(" and ");
            res.push_str(m[len - 1].as_ref());
            res
        }
    }
}

pub fn insert_missing_guilds(ctx: &Context) {
    use diesel;
    use diesel::prelude::*;
    use models::NewGuild;
    use schema::guild;
    use PgConnectionManager;

    let pool = extract_pool!(&ctx);

    let guilds: Vec<_> = log_time!(
        with_cache(|c| c
            .all_guilds()
            .iter()
            .map(|&g| NewGuild { id: g.0 as i64 })
            .collect()),
        "with_cache: find_new_guilds"
    );

    diesel::insert_into(guild::table)
        .values(&guilds)
        .on_conflict_do_nothing()
        .execute(pool)
        .expect("Error building any missing guilds.");
}

pub struct HistoryIterator {
    last_id: Option<MessageId>,
    channel: ChannelId,
    message_vec: Vec<Message>,
}

/// An iterator over discord messages, runs forever through all the messages in a channel's history
impl HistoryIterator {
    pub fn new(c_id: ChannelId) -> Self {
        HistoryIterator {
            last_id: None,
            channel: c_id,
            message_vec: Vec::new(),
        }
    }
}

impl Iterator for HistoryIterator {
    type Item = Message;
    fn next(&mut self) -> Option<Message> {
        // no messages, get some more
        if self.message_vec.is_empty() {
            match self.channel.messages(|g| match self.last_id {
                Some(id) => g.before(id),
                None => g,
            }) {
                Ok(messages) => {
                    if messages.is_empty() {
                        // no more messages to get, end iterator here
                        return None;
                    }
                    self.message_vec.extend(messages);
                    self.last_id = self.message_vec.last().map(|m| m.id);
                }
                Err(why) => {
                    if let serenity::Error::Http(HttpError::UnsuccessfulRequest(ref resp)) = why {
                        if resp.status.is_server_error() {
                            // haha yes, just try again
                            // thanks discord
                            return self.next();
                        }
                    }
                    // any other error and we should just stop here
                    return None;
                }
            }
        }

        let m = self.message_vec.pop();

        // we should only be ending the iterator if there are no more messages
        // upon which we would have exited earlier
        if m.is_none() {
            panic!("Messages didn't exist? aborting.");
        }

        m
    }
}

pub fn try_resolve_user(s: &str, g_id: GuildId) -> Result<Member, ()> {
    if let Some(g) = g_id.to_guild_cached() {
        let guild = g.read();

        if let Ok(u) = s.parse::<UserId>() {
            return guild.member(u).map_err(|_| ());
        }

        return guild.member_named(s).cloned().ok_or(());
    } else {
        return Err(());
    }
}

pub fn nsfw_check(
    _: &mut Context,
    msg: &Message,
    _: &mut Args,
    _: &CommandOptions,
) -> Result<(), String> {
    if msg
        .channel_id
        .to_channel_cached()
        .map_or(false, |c| c.is_nsfw())
    {
        Ok(())
    } else {
        Err("Channel is not NSFW".to_owned())
    }
}

#[must_use]
pub fn send_message<F, C: Into<ChannelId> + Copy>(chan_id: C, f: F) -> serenity::Result<()>
where
    F: FnOnce(CreateMessage) -> CreateMessage,
{
    use rmp_serde::Serializer;
    use serde::Serialize;
    use std::io::Write;
    use {connect_socket, MESSENGER_SOCKET};

    let msg = f(CreateMessage::default());
    let map = serenity::utils::vecmap_to_json_map(msg.0);

    Message::check_content_length(&map)?;
    Message::check_embed_length(&map)?;

    let object = &serde_json::Value::Object(map);
    let content = serde_json::to_string(&object)?;

    let mut buf = Vec::new();

    (chan_id.into(), content)
        .serialize(&mut Serializer::new(&mut buf))
        .unwrap();

    let use_fallback = {
        let mut socket = MESSENGER_SOCKET.lock();

        if socket.is_none() {
            *socket = connect_socket();
        } // try to connect once

        match socket.as_mut() {
            Some(skt) => skt.write_all(buf.as_slice()).is_err(),
            _ => true,
        }
    };

    if use_fallback {
        // manual print since logging results in a message being sent
        eprintln!("[WARN][bot] Using builtin message sender.");
        serenity::http::send_message(chan_id.into().0, &object)?;
    }

    Ok(())
}

#[must_use]
pub fn say<D: Display, C: Into<ChannelId> + Copy>(chan_id: C, content: D) -> serenity::Result<()> {
    send_message(chan_id.into(), |m| m.content(content))
}

pub fn get_random_members(guild_id: GuildId) -> Option<Vec<Member>> {
    guild_id.to_guild_cached().and_then(|g| {
        let guild = g.read();
        let member_ids: Vec<_> = guild
            .members
            .keys()
            .filter(|u| // no bots thanks
                         match u.to_user_cached() {
                             Some(user) => !user.read().bot,
                             None       => false,
                         })
            .collect();
        let &&member_id = member_ids.choose(&mut thread_rng())?;
        guild.member(member_id).ok().map(|m| vec![m.clone()])
    })
}

// Stolen from serenity source
pub fn normalize(text: &str) -> String {
    // Remove invite links and popular scam websites, mostly to prevent the
    // current user from triggering various ad detectors and prevent embeds.
    text.replace("discord.gg", "discord\u{2024}gg")
        .replace("discord.me", "discord\u{2024}me")
        .replace("discordlist.net", "discordlist\u{2024}net")
        .replace("discordservers.com", "discordservers\u{2024}com")
        .replace("discordapp.com/invite", "discordapp\u{2024}com/invite")
        // Remove right-to-left override and other similar annoying symbols
        .replace('\u{202E}', " ") // RTL Override
        .replace('\u{200F}', " ") // RTL Mark
        .replace('\u{202B}', " ") // RTL Embedding
        .replace('\u{200B}', " ") // Zero-width space
        .replace('\u{200D}', " ") // Zero-width joiner
        .replace('\u{200C}', " ") // Zero-width non-joiner
        // Remove everyone and here mentions. Has to be put after ZWS replacement
        // because it utilises it itself.
        .replace("@everyone", "@\u{200B}everyone")
        .replace("@here", "@\u{200B}here")
}

pub fn with_pool<T, F>(ctx: &Context, f: F) -> T
where
    F: Fn(PooledConnection<ConnectionManager<PgConnection>>) -> T,
{
    let pool = ctx
        .data
        .lock()
        .get::<super::PgConnectionManager>()
        .unwrap()
        .get()
        .unwrap();

    f(pool)
}
