#[path = "../common/mod.rs"]
mod common;

#[cfg(deloist_private)]
#[path = "../../private/tests/account.rs"]
mod account;
mod assets;
mod attach;
mod bot_management;
mod branch;
mod catalog;
mod connectors;
mod dates;
mod discord;
mod features;
#[cfg(deloist_private)]
#[path = "../../private/tests/feedback.rs"]
mod feedback;
mod habits;
mod handoff;
mod hermes_import;
mod know_me;
mod limits;
mod memory;
mod mentions;
mod optimizer;
mod night;
mod pets;
mod queue;
mod reasons;
mod reminders;
mod routines_chat;
mod runs;
mod slack;
mod specialist_offer;
mod team_async;
mod team_proposal;
mod team_rooms;
mod telegram;
mod todos;
mod toast_reply;
mod workflows;
