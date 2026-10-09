mod agent;
mod capture;
mod codex;
mod copilot;
mod deck;
mod env;
mod error;
mod html;
mod lint;
mod mcp;
mod narration;
mod protocol;
mod providers;
mod review;
mod speech;
mod templates;
mod watcher;

use serde::Serialize;
use tauri::{AppHandle, State};

use agent::{AgentManager, SendArgs};
use deck::{Deck, DeckSummary};
use error::Result;
use watcher::DeckWatcher;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreatedSlide {
    deck: Deck,
    slide: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdatedSlide {
    deck: Deck,
    /// The slide's markup before the update, to undo it.
    previous: String,
}

#[tauri::command]
async fn list_decks(app: AppHandle) -> Result<Vec<DeckSummary>> {
    // Documents access can wait for macOS permission/cloud services. Keep the
    // window's event loop free while the filesystem resolves that request.
    tauri::async_runtime::spawn_blocking(move || deck::list(&deck::library_root(&app)?))
        .await
        .map_err(|e| error::Error::msg(e.to_string()))?
}

#[tauri::command]
fn create_deck(
    app: AppHandle,
    watcher: State<DeckWatcher>,
    title: String,
    template: Option<String>,
) -> Result<Deck> {
    let root = templates::user_root(&app)?;
    let source = template
        .as_deref()
        .map(|t| templates::source(&root, t))
        .transpose()?;
    let template = template
        .as_deref()
        .zip(source.as_deref())
        .map(|(id, html)| deck::TemplateSource { id, html });
    let deck = deck::create(&deck::library_root(&app)?, &title, template)?;
    stage_deck_template(&app, &deck);
    watcher.watch(app, deck.id.clone(), deck.path.clone().into())?;
    Ok(deck)
}

/// Copies the deck's template into its internals, where the agent reads its layouts.
/// Best effort: a template that is gone only means there are no layouts to read.
fn stage_deck_template(app: &AppHandle, deck: &Deck) {
    if let (Some(template), Ok(root)) = (&deck.template, templates::user_root(app)) {
        let _ = templates::stage(std::path::Path::new(&deck.path), &root, template);
    }
}

#[tauri::command]
fn list_templates(app: AppHandle) -> Result<Vec<templates::TemplateSummary>> {
    Ok(templates::list(&templates::user_root(&app)?))
}

/// Copies a template into the deck's internals for the agent; returns its deck-relative path.
#[tauri::command]
fn stage_template(app: AppHandle, id: String, template: String) -> Result<String> {
    templates::stage(
        &deck::deck_dir(&app, &id)?,
        &templates::user_root(&app)?,
        &template,
    )
}

/// Gives a deck without slides the template's styles.
#[tauri::command]
fn apply_template(app: AppHandle, id: String, template: String) -> Result<Deck> {
    let root = templates::user_root(&app)?;
    let html = templates::source(&root, &template)?;
    let source = deck::TemplateSource {
        id: &template,
        html: &html,
    };
    let deck = deck::apply_template(&deck::deck_dir(&app, &id)?, &id, source)?;
    stage_deck_template(&app, &deck);
    Ok(deck)
}

/// Adds a copy of one of the template's slides after `after`.
#[tauri::command]
fn add_template_slide(
    app: AppHandle,
    id: String,
    template: String,
    slide: String,
    after: Option<String>,
) -> Result<CreatedSlide> {
    let html = templates::source(&templates::user_root(&app)?, &template)?;
    let (deck, slide) =
        deck::add_template_slide(&deck::deck_dir(&app, &id)?, &id, after, &html, &slide)?;
    Ok(CreatedSlide { deck, slide })
}

/// Saves the deck as a new user template, with placeholder text in place of its content.
#[tauri::command]
fn create_template(app: AppHandle, id: String, name: String) -> Result<templates::TemplateSummary> {
    templates::create_from_deck(
        &deck::deck_dir(&app, &id)?,
        &templates::user_root(&app)?,
        &name,
    )
}

#[tauri::command]
fn open_deck(
    app: AppHandle,
    agent: State<AgentManager>,
    watcher: State<DeckWatcher>,
    id: String,
) -> Result<Deck> {
    let deck = deck::open(&deck::deck_dir(&app, &id)?, &id, !agent.is_running(&id))?;
    stage_deck_template(&app, &deck);
    watcher.watch(app, deck.id.clone(), deck.path.clone().into())?;
    Ok(deck)
}

#[tauri::command]
fn close_deck(watcher: State<DeckWatcher>) {
    watcher.stop();
}

#[tauri::command]
fn load_deck(app: AppHandle, id: String) -> Result<Deck> {
    deck::load(&deck::deck_dir(&app, &id)?, &id)
}

#[tauri::command]
fn load_narration(app: AppHandle, id: String) -> Result<narration::Document> {
    narration::load(&deck::deck_dir(&app, &id)?)
}

#[tauri::command]
fn save_narration(
    app: AppHandle,
    id: String,
    manifest: narration::Manifest,
    base: String,
) -> Result<narration::Document> {
    narration::save(&deck::deck_dir(&app, &id)?, manifest, &base)
}

#[tauri::command]
fn save_review(app: AppHandle, id: String, review: review::Review) -> Result<()> {
    deck::save_review(&deck::deck_dir(&app, &id)?, &review)
}

#[tauri::command]
fn speech_status(app: AppHandle, manager: State<speech::SpeechManager>) -> Result<speech::Status> {
    manager.status(&app)
}
#[tauri::command]
fn speech_takes(
    app: AppHandle,
    id: String,
) -> Result<std::collections::BTreeMap<String, speech::cache::Take>> {
    speech::takes(&app, &id)
}
#[tauri::command]
async fn install_speech_pack(
    app: AppHandle,
    manager: State<'_, speech::SpeechManager>,
    job_id: String,
    source: Option<String>,
    provider_id: String,
) -> Result<()> {
    manager.install(app, job_id, provider_id, source).await
}
#[tauri::command]
async fn remove_speech_pack(
    app: AppHandle,
    manager: State<'_, speech::SpeechManager>,
    provider_id: String,
) -> Result<()> {
    manager.remove(&app, &provider_id).await
}
#[tauri::command]
async fn generate_speech(
    app: AppHandle,
    manager: State<'_, speech::SpeechManager>,
    job_id: String,
    id: String,
    slide: Option<String>,
) -> Result<speech::GenerationResult> {
    manager.generate(app, job_id, id, slide).await
}
#[tauri::command]
fn cancel_speech(manager: State<speech::SpeechManager>, job_id: String) -> Result<()> {
    manager.cancel(&job_id)
}

#[tauri::command]
fn rename_deck(app: AppHandle, id: String, title: String) -> Result<Deck> {
    deck::rename(&deck::deck_dir(&app, &id)?, &id, &title)
}

#[tauri::command]
fn delete_deck(
    app: AppHandle,
    agent: State<AgentManager>,
    watcher: State<DeckWatcher>,
    id: String,
) -> Result<()> {
    agent.interrupt(&id);
    watcher.stop();
    deck::delete_deck(&deck::deck_dir(&app, &id)?)
}

#[tauri::command]
fn reorder_slides(app: AppHandle, id: String, slides: Vec<String>) -> Result<Deck> {
    deck::reorder(&deck::deck_dir(&app, &id)?, &id, slides)
}

#[tauri::command]
fn add_slide(app: AppHandle, id: String, after: Option<String>) -> Result<CreatedSlide> {
    let (deck, slide) = deck::add_blank(&deck::deck_dir(&app, &id)?, &id, after)?;
    Ok(CreatedSlide { deck, slide })
}

#[tauri::command]
fn duplicate_slide(app: AppHandle, id: String, slide: String) -> Result<CreatedSlide> {
    let (deck, slide) = deck::duplicate(&deck::deck_dir(&app, &id)?, &id, &slide)?;
    Ok(CreatedSlide { deck, slide })
}

#[tauri::command]
fn set_slide_hidden(app: AppHandle, id: String, slide: String, hidden: bool) -> Result<Deck> {
    deck::set_slide_hidden(&deck::deck_dir(&app, &id)?, &id, &slide, hidden)
}

#[tauri::command]
fn set_slide_locked(app: AppHandle, id: String, slide: String, locked: bool) -> Result<Deck> {
    deck::set_slide_locked(&deck::deck_dir(&app, &id)?, &id, &slide, locked)
}

#[tauri::command]
fn add_section(app: AppHandle, id: String, before: Option<String>, title: String) -> Result<Deck> {
    deck::add_section(&deck::deck_dir(&app, &id)?, &id, before, &title)
}

#[tauri::command]
fn rename_section(app: AppHandle, id: String, index: usize, title: String) -> Result<Deck> {
    deck::rename_section(&deck::deck_dir(&app, &id)?, &id, index, &title)
}

#[tauri::command]
fn delete_section(app: AppHandle, id: String, index: usize) -> Result<Deck> {
    deck::delete_section(&deck::deck_dir(&app, &id)?, &id, index)
}

#[tauri::command]
fn delete_slide(app: AppHandle, id: String, slide: String) -> Result<Deck> {
    deck::delete_slide(&deck::deck_dir(&app, &id)?, &id, &slide)
}

/// Saves a slide edited on the stage (text edits, moved elements). `base` is its hash when
/// the edit started.
#[tauri::command]
fn update_slide(
    app: AppHandle,
    id: String,
    slide: String,
    markup: String,
    base: String,
) -> Result<UpdatedSlide> {
    let (deck, previous) =
        deck::update_slide(&deck::deck_dir(&app, &id)?, &id, &slide, &markup, &base)?;
    Ok(UpdatedSlide { deck, previous })
}

#[tauri::command]
fn save_deck_source(
    app: AppHandle,
    agent: State<AgentManager>,
    id: String,
    source: String,
    base: Option<String>,
) -> Result<Deck> {
    // Normalizing mid-turn could rewrite ids the agent is about to reference.
    let normalize = !agent.is_running(&id);
    deck::save_source(
        &deck::deck_dir(&app, &id)?,
        &id,
        &source,
        base.as_deref(),
        normalize,
    )
}

#[tauri::command]
fn import_assets(app: AppHandle, id: String, paths: Vec<String>) -> Result<Vec<String>> {
    deck::import_assets(&deck::deck_dir(&app, &id)?, paths)
}

#[tauri::command]
fn save_asset(app: AppHandle, id: String, name: String, data: String) -> Result<String> {
    deck::save_asset(&deck::deck_dir(&app, &id)?, &name, &data)
}

#[tauri::command]
fn export_deck(app: AppHandle, id: String, dest: String) -> Result<()> {
    deck::export(&deck::deck_dir(&app, &id)?, std::path::Path::new(&dest))
}

#[tauri::command]
fn lint_deck(app: AppHandle, id: String) -> Result<Vec<lint::Issue>> {
    deck::lint(&deck::deck_dir(&app, &id)?)
}

/// Screenshots `rect` of the window (the sketched-on slide) into the deck's internals.
/// Both are in CSS pixels; `viewport` is the window's size, to find the display scale.
#[tauri::command]
async fn capture_sketch(
    app: AppHandle,
    webview: tauri::Webview,
    id: String,
    rect: capture::Rect,
    viewport: capture::Size,
) -> Result<String> {
    let dir = deck::deck_dir(&app, &id)?;
    let png = capture::snapshot(&webview, rect, viewport, capture::SKETCH_WIDTH).await?;
    deck::save_sketch(&dir, &png)
}

/// Creates `<parent>/<deck title>` (or `<deck title> 2`, …) for exported slide images.
#[tauri::command]
fn create_image_export_dir(app: AppHandle, id: String, parent: String) -> Result<String> {
    let deck = deck::load(&deck::deck_dir(&app, &id)?, &id)?;
    let dir = deck::create_export_dir(std::path::Path::new(&parent), &deck.title)?;
    Ok(dir.to_string_lossy().into_owned())
}

/// Screenshots `rect` of the window (one slide, shown full size) as `<dir>/<slide-NN>.png`.
#[tauri::command]
async fn export_slide_image(
    webview: tauri::Webview,
    dir: String,
    index: usize,
    total: usize,
    rect: capture::Rect,
    viewport: capture::Size,
) -> Result<String> {
    let png = capture::snapshot(&webview, rect, viewport, capture::SLIDE_WIDTH).await?;
    let path = std::path::Path::new(&dir).join(deck::slide_image_name(index, total));
    std::fs::write(&path, png)?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
fn load_chat(app: AppHandle, id: String) -> Result<serde_json::Value> {
    deck::load_chat(&deck::deck_dir(&app, &id)?)
}

#[tauri::command]
fn save_chat(app: AppHandle, id: String, chat: serde_json::Value) -> Result<()> {
    deck::save_chat(&deck::deck_dir(&app, &id)?, &chat)
}

#[tauri::command]
fn reset_chat(app: AppHandle, agent: State<AgentManager>, id: String) -> Result<()> {
    agent.interrupt(&id);
    let dir = deck::deck_dir(&app, &id)?;
    for provider in agent::Provider::ALL {
        deck::write_session(&dir, provider.session_file(), None)?;
    }
    deck::save_chat(&dir, &serde_json::Value::Null)
}

#[tauri::command]
fn send_message(app: AppHandle, agent: State<AgentManager>, args: SendArgs) -> Result<()> {
    agent.send(app, args)
}

#[tauri::command]
async fn codex_permission_modes(app: AppHandle, id: String) -> Result<Vec<codex::PermissionMode>> {
    let dir = deck::deck_dir(&app, &id)?;
    codex::permission_modes(&dir).await
}

#[tauri::command]
fn respond_approval(
    agent: State<AgentManager>,
    deck_id: String,
    id: String,
    decision: codex::Decision,
) -> Result<()> {
    agent.approvals.respond(&deck_id, &id, decision)
}

#[tauri::command]
fn interrupt_agent(agent: State<AgentManager>, id: String) {
    agent.interrupt(&id);
}

#[tauri::command]
fn agent_running(agent: State<AgentManager>, id: String) -> bool {
    agent.is_running(&id)
}

#[tauri::command]
async fn list_providers() -> Vec<providers::ProviderInfo> {
    providers::list().await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // `slopslide --lint-mcp <deck dir>`: the agent's lint tool, started by Claude Code.
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some(mcp::FLAG) {
        let dir = args
            .get(2)
            .map_or_else(|| ".".into(), std::path::PathBuf::from);
        return mcp::serve(&dir);
    }
    env::adopt_login_shell_path();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AgentManager::default())
        .manage(DeckWatcher::default())
        .manage(speech::SpeechManager::default())
        .register_uri_scheme_protocol("slop", |ctx, request| {
            protocol::handle(ctx.app_handle(), request)
        })
        .invoke_handler(tauri::generate_handler![
            list_decks,
            create_deck,
            open_deck,
            close_deck,
            load_deck,
            load_narration,
            save_narration,
            speech_status,
            speech_takes,
            install_speech_pack,
            remove_speech_pack,
            generate_speech,
            cancel_speech,
            rename_deck,
            save_review,
            delete_deck,
            reorder_slides,
            add_slide,
            duplicate_slide,
            set_slide_hidden,
            set_slide_locked,
            add_section,
            rename_section,
            delete_section,
            delete_slide,
            update_slide,
            save_deck_source,
            import_assets,
            save_asset,
            export_deck,
            lint_deck,
            capture_sketch,
            create_image_export_dir,
            export_slide_image,
            load_chat,
            save_chat,
            reset_chat,
            send_message,
            interrupt_agent,
            codex_permission_modes,
            respond_approval,
            agent_running,
            list_providers,
            list_templates,
            stage_template,
            apply_template,
            add_template_slide,
            create_template,
        ])
        .run(tauri::generate_context!())
        .expect("error while running SlopSlide");
}
