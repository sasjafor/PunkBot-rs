mod clear;
mod common;
mod disconnect;
mod filter;
mod loop_track;
mod now_playing;
mod pause;
mod play;
mod queue;
mod remove;
mod resume;
mod search;
mod seek;
mod shuffle;
mod skip;
mod stop;
mod summon;
mod volume;

pub use clear::clear;
pub use disconnect::disconnect;
pub use filter::filter;
pub use loop_track::loop_track;
pub use now_playing::now_playing;
pub use pause::pause;
pub use play::play;
pub use queue::queue;
pub use remove::remove;
pub use resume::resume;
pub use search::search;
pub use seek::seek;
pub use shuffle::shuffle;
pub use skip::skip;
pub use stop::stop;
pub use summon::summon;
pub use volume::volume;

pub fn all() -> Vec<poise::Command<crate::Data, crate::PError>> {
    vec![
        play(),
        now_playing(),
        summon(),
        disconnect(),
        pause(),
        resume(),
        skip(),
        stop(),
        loop_track(),
        remove(),
        clear(),
        shuffle(),
        volume(),
        filter(),
        seek(),
        queue(),
        search(),
    ]
}
