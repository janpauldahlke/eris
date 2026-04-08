//! Agenda-linked alarm persistence, background firing, and startup overdue hints.

pub mod missed_startup;
pub mod scheduler;

pub use missed_startup::startup_overdue_agenda_hint;
pub use scheduler::spawn_alarm_scheduler;
