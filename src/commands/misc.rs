use serenity::{
    prelude::*,
    framework::standard::{
        StandardFramework,
    },
    utils::{
        with_cache,
    },
};
use chrono::Utc;
use procinfo;
use std::time;
use rand;
use rand::Rng;
use utils::{try_resolve_user};
use itertools::Itertools;
use whirlpool::{Whirlpool, Digest};
use std::num::Wrapping;


fn process_usage() -> f64 {
    use std::thread;
    let start_measure = procinfo::pid::stat_self().unwrap().utime;
    thread::sleep(time::Duration::from_millis(100));
    let end_measure = procinfo::pid::stat_self().unwrap().utime;

    let diff = end_measure - start_measure;
    return diff as f64 / 0.1; // util seconds / 100ms per second
}


command!(status_cmd(ctx, msg) {
    use ::{StartTime, CmdCounter};

    let mem_usage = procinfo::pid::statm_self().ok().map_or(0, |p| p.resident);
    let cpu_usage = process_usage();
    let uptime = {
        let &start = ctx.data.lock().get::<StartTime>().unwrap();
        let now = Utc::now().naive_utc();

        now.signed_duration_since(start)
    };

    let cmd_count = {
        let lock = ctx.data.lock();
        let count = *lock.get::<CmdCounter>().unwrap().read().unwrap();
        count
    };

    let (g_c, c_c, u_c, s_c) = with_cache(
        |c| {
            let g_c = c.guilds.len();
            let c_c = c.channels.len();
            let u_c = c.users.len();
            let s_c = c.shard_count;
            (g_c, c_c, u_c, s_c)
        });

    let (u_days, u_hours, u_min, u_sec) = (
        uptime.num_days(),
        uptime.num_hours() % 24,
        uptime.num_minutes() % 60,
        uptime.num_seconds() % 60,
    );

    let uptime_str = format!("{}d, {}h, {}m, {}s", u_days, u_hours, u_min, u_sec);

    msg.channel_id.send_message(
        |m| m.embed(
            |e| e
                .title("genericbot stats")
                .colour(0x2C78C8)
                .field("Uptime", uptime_str, true)
                .field("Guild count", g_c, true)
                .field("Channel count", c_c, true)
                .field("User count", u_c, true)
                .field("Commands executed", cmd_count, true)
                .field("Shard count", s_c, true)
                .field("Cpu usage", format!("{:.1}%", cpu_usage), true)
                .field("Mem usage", format!("{:.2}MB", mem_usage), true)
        ))?;
});


command!(q(_ctx, msg) {
    void!(msg.channel_id.say(rand::thread_rng()
                             .choose(&["Yes", "No"])
                             .unwrap()));
});


command!(message_owner(ctx, _msg, args) {
    use ::OwnerId;
    let text = args.full();

    let lock = ctx.data.lock();
    let user = &lock.get::<OwnerId>().unwrap();
    user.direct_message(|m| m.content(text))?;
});


macro_rules! x_someone {
    ( $name:ident, $send_msg:expr, $err:expr ) => (
        command!($name(_ctx, msg, args) {
            let users: Vec<_> = args.multiple_quoted::<String>()
                .map(|u| u.into_iter()
                     .filter_map(|s| try_resolve_user(&s, msg.guild_id().unwrap()).ok())
                     .collect())
                .unwrap_or_else(|_| Vec::new());

            let res = if !users.is_empty() {
                let mention_list = users.into_iter().map(|u| u.mention()).join(", ");
                format!($send_msg, msg.author.mention(), mention_list)
            } else {
                $err.to_string()
            };

            msg.channel_id.say(res)?;
        });
    )
}


x_someone!(hug, "{} hugs {}!", "You can't hug nobody!");
x_someone!(slap, "{} slaps {}! B..Baka!!!", "Go slap yourself you baka");
x_someone!(kiss, "{} Kisses {}! Chuuuu!", "DW anon you'll find someone to love some day!");


command!(rate(_ctx, msg, args) {
    let asked = args.full().trim();
    let result = Whirlpool::digest_str(&asked);
    let sum: Wrapping<u8> = result.into_iter().map(Wrapping).sum();

    let modulus = sum % Wrapping(12);

    void!(msg.channel_id.say(format!("I rate {}: {}/10", asked, modulus)));
});


pub fn setup_misc(_client: &mut Client, frame: StandardFramework) -> StandardFramework {
    frame.group("Misc",
                |g| g
                .command("stats", |c| c
                         .cmd(status_cmd)
                         .desc("Bot stats")
                         .batch_known_as(&["status"])
                )
                .command("q", |c| c
                         .cmd(q)
                         .desc("Ask a question")
                )
                .command("message_owner", |c| c
                         .cmd(message_owner)
                         .desc("Send a message to the bot owner.")
                )
                .command("hug", |c| c
                         .cmd(hug)
                         .desc("Hug someone")
                         .guild_only(true)
                )
                .command("slap", |c| c
                         .cmd(slap)
                         .desc("Slap a bitch")
                         .guild_only(true)
                )
                .command("kiss", |c| c
                         .cmd(kiss)
                         .desc("Kiss someone")
                         .guild_only(true)
                )
                .command("rate", |c| c
                         .cmd(rate)
                         .desc("Rate something."))
    )
}
