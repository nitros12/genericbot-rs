use serenity::{
    model::{
        id::{GuildId, UserId, MessageId, ChannelId},
        channel::Message,
    },
    framework::standard::{
        Args,
        CommandOptions,
    },
};
use serenity::prelude::*;

#[macro_use]
pub mod macros;
pub mod markov;


pub fn names_for_members<U, G>(u_ids: &[U], g_id: G) -> Vec<String>
    where U: Into<UserId> + Copy,
          G: Into<GuildId> + Copy,
{
    use serenity::{
        utils::with_cache,
    };

    fn backup_getter<U>(u_id: U) -> String
        where U: Into<UserId> + Copy,
    {
        match u_id.into().get() {
            Ok(u) => u.name,
            _     => u_id.into().to_string(),
        }
    }

    with_cache(
        |cache| cache.guild(g_id).map(|g| {
            let members = &g.read().members;
            u_ids.iter().map(
                |&id| members.get(&id.into()).map_or_else(
                    || backup_getter(id),
                    |m| m.display_name().to_string()))
                           .collect()
        })).unwrap_or_else(|| u_ids.iter().map(|&id| backup_getter(id)).collect())
}


pub fn and_comma_split<T: AsRef<str>>(m: &[T]) -> String {
    let mut res = String::new();
    let end = m.len() as isize;

    for (n, s) in m.into_iter().enumerate() {
        res.push_str(s.as_ref());
        if n as isize == end - 2 {
            res.push_str(" and ");
        } else if (n as isize) < end - 2 {
            res.push_str(", ");
        }
    }
    return res;
}


pub fn insert_missing_guilds(ctx: &Context) {
    use diesel;
    use diesel::prelude::*;
    use models::NewGuild;
    use schema::guild;
    use ::PgConnectionManager;
    use serenity::utils::with_cache;

    let pool = extract_pool!(&ctx);

    let guilds: Vec<_> = with_cache(|c| c.all_guilds().iter().map(
        |&g| NewGuild { id: g.0 as i64 }
    ).collect());

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
        HistoryIterator { last_id: None, channel: c_id, message_vec: Vec::new() }
    }
}


impl Iterator for HistoryIterator {
    type Item = Message;
    fn next(&mut self) -> Option<Message> {
        // no messages, get some more
        if self.message_vec.is_empty() {
            match self.channel.messages(
                |g| match self.last_id {
                    Some(id) => g.before(id),
                    None     => g
                }) {
                Ok(messages) => {
                    if messages.is_empty() {
                        // no more messages to get, end iterator here
                        return None;
                    }
                    self.message_vec.extend(messages);
                    self.last_id = self.message_vec.last().map(|m| m.id);
                },
                Err(why) => panic!(format!("Couldn't get messages: {}, aborting.", why)),
            }
        }

        let m = self.message_vec.pop();
        if m.is_none() {
            panic!("Messages didn't exist? aborting.");
        }
        return m;
    }
}


pub fn try_resolve_user(s: &str, g_id: GuildId) -> Result<UserId, ()> {
    if let Ok(u) = s.parse::<UserId>() {
        return Ok(u);
    }

    if let Some(g) = g_id.find() {
        let guild = g.read();

        if let Some(m) = guild.member_named(s) {
            let uid = m.user.read();
            return Ok(uid.id);
        } else {
            return Err(());
        }
    } else {
        return Err(());
    }
}


pub fn nsfw_check(_: &mut Context, msg: &Message, _: &mut Args, _: &CommandOptions) -> bool {
    msg.channel_id.find().map_or(false, |c| c.is_nsfw())
}