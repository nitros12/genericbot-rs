pub mod schema;
pub mod models;

mod commands;

#[macro_use]
extern crate serenity;
extern crate dotenv;
#[macro_use]
extern crate diesel;
extern crate r2d2;
extern crate r2d2_diesel;
extern crate chrono;
extern crate typemap;
extern crate threadpool;

use serenity::{
    CACHE,
    prelude::*,
    model::{
        guild::Guild,
        channel::Message,
        gateway::Ready,
    },
    client::bridge::gateway::{ShardManager},
    framework::standard::StandardFramework
};

use diesel::{
    prelude::*,
    pg::PgConnection,
};
use r2d2_diesel::ConnectionManager;

use std::sync::Arc;
use typemap::Key;

struct Handler;

impl EventHandler for Handler {
    fn ready(&self, _: Context, ready: Ready) {
        if let Some(shard) = ready.shard {
            println!("Connected as: {} on shard {} of {}", ready.user.name, shard[0], shard[1]);
        }
    }

    fn guild_create(&self, ctx: Context, guild: Guild, _: bool) {
        use schema::{guild, prefix};
        use models::{Guild, NewPrefix};

        let data = ctx.data.lock();
        let pool = &*data.get::<PgConnectionManager>().unwrap().get().unwrap();
        drop(data);

        let new_guild = Guild {
            id: guild.id.0 as i64,
            markov_on: false,
            tag_prefix_on: false,
            commands_from: 0,
        };

        let default_prefix = NewPrefix {
            guild_id: guild.id.0 as i64,
            pre: "#!",
        };

        diesel::insert_into(guild::table)
            .values(&new_guild)
            .on_conflict_do_nothing()
            .execute(pool)
            .expect("Couldn't create guild");

        diesel::insert_into(prefix::table)
            .values(&default_prefix)
            .on_conflict_do_nothing()
            .execute(pool)
            .expect("Couldn't create default prefix");
    }
}


struct ShardManagerContainer;

impl Key for ShardManagerContainer {
    type Value = Arc<Mutex<ShardManager>>;
}

struct PgConnectionManager;

impl Key for PgConnectionManager {
    type Value = r2d2::Pool<ConnectionManager<PgConnection>>;
}


fn get_prefixes(ctx: &mut Context, m: &Message) -> Option<Arc<Vec<String>>> {
    use models::Prefix;
    use schema::prefix::dsl::*;

    let data = ctx.data.lock();
    let pool = &*data.get::<PgConnectionManager>().unwrap().get().unwrap();

    drop(data);

    if let Some(g_id) = m.guild_id() {
        let prefixes = prefix.filter(guild_id.eq(g_id.0 as i64))
            .load::<Prefix>(pool)
            .expect("Error loading prefixes")
            .into_iter()
            .map(|p| p.pre)
            .collect();
        Some(Arc::new(prefixes))
    } else {
        None
    }
}

// Our setup stuff
fn setup(client: &mut Client, frame: StandardFramework) -> StandardFramework {
    use serenity::framework::standard::{
        DispatchError::*,
        help_commands,
        HelpBehaviour,
    };

    frame.on_dispatch_error(| _, msg, err | {
        if let Some(s) = match err {
            OnlyForGuilds =>
                Some("This command can only be used in private messages.".to_string()),
            NotEnoughArguments { min, given } =>
                Some(format!("Command missing required arguments, given: {} but needs {}.", given, min)),
            TooManyArguments {max, given} =>
                Some(format!("Command given too many arguments, given: {} but takes at most {}.", given, max)),
            RateLimited(time) =>
                Some(format!("You are ratelimited, try again in: {} seconds.", time)),
            CheckFailed =>
                Some("The check for this command failed.".to_string()),
            LackOfPermissions(perms) =>
                Some(format!("This command requires permissions: {:?}", perms)),
            _ => None,
        } {
            let _ = msg.channel_id.say(&s);
        }})
         .after(| ctx, msg, _, err | {
             use schema::guild::dsl::*;

             if let Some(g_id) = msg.guild_id() {
                if !err.is_err() {
                    let data = ctx.data.lock();
                    let pool = &*data.get::<PgConnectionManager>().unwrap().get().unwrap();
                    drop(data);

                    diesel::update(guild.find(g_id.0 as i64))
                        .set(commands_from.eq(commands_from + 1))
                        .execute(pool)
                        .unwrap();

                }
             }
         })
        .configure(|c| c
                   .dynamic_prefixes(get_prefixes)
                   .prefix("#!"))
        .customised_help(help_commands::plain, |c| c
                         .individual_command_tip(
                             "To get help on a specific command, pass the command name as an argument to help.")
                         .command_not_found_text("A command with the name {} does not exist.")
                         .suggestion_text("This command was not, maybe you meant: {}?")
                         .lacking_permissions(HelpBehaviour::Hide))
}


pub fn log_message(msg: &String) {
    use serenity::model::channel::Channel::Guild;

    let chan_id = dotenv::var("DISCORD_BOT_LOG_CHAN").unwrap().parse::<u64>().unwrap();
    if let Some(Guild(chan)) = CACHE.read().channel(chan_id) {
        chan.read().say(msg).unwrap();
    }
}


fn main() {
    let token = dotenv::var("DISCORD_BOT_TOKEN").unwrap();
    let db_url = dotenv::var("DISCORD_BOT_DB").unwrap();

    let manager = ConnectionManager::<PgConnection>::new(db_url);
    let pool = r2d2::Pool::builder().build(manager).unwrap();

    let mut client = Client::new(&token, Handler).unwrap();

    let setup_fns = &[setup, commands::tags::setup_tags];

    let framework = setup_fns.iter().fold(
        StandardFramework::new(),
        | acc, fun | fun(&mut client, acc));


    client.with_framework(framework);

    {
        let mut data = client.data.lock();
        data.insert::<ShardManagerContainer>(Arc::clone(&client.shard_manager));
        data.insert::<PgConnectionManager>(pool);
    }


    if let Err(why) = client.start_autosharded() {
        println!("AAA: {:?}", why);
    }
}
