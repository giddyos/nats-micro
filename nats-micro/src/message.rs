/// Marker implemented by `#[message]` JSON wire types.
pub trait JsonMessage {
    const NAME: &'static str;
}
