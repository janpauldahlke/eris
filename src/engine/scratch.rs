use ollama_rs::generation::chat::{ChatMessageFinalResponseData};
fn t(data: ChatMessageFinalResponseData) {
    let _x: std::string::String = data.prompt_eval_count;
}
