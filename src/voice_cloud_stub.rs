//! Public build: cloud speech recognition (deloist.ru) is not part of this repository.
//! Local and own-server recognition (`stt_local`) work as usual.

use crate::voice::Clip;
use crate::App;

pub async fn transcribe(_app: &App, _clip: Clip) -> Result<String, (u16, &'static str)> {
    Err((503, "Облачное распознавание речи недоступно в этой сборке. Выбери распознавание на компьютере или свой сервер"))
}
