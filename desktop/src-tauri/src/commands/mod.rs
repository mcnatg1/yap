mod history;
mod live;
pub(crate) mod media_protocol;
mod setup;

pub(crate) fn register(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    let job_resources = std::sync::Arc::new(
        crate::jobs::RecordingJobResources::open_default()
            .expect("recording job resources must open before commands are registered"),
    );
    let recording_jobs = crate::jobs::commands::RecordingJobs::from_default_resources(
        std::sync::Arc::clone(&job_resources),
    );
    let remote_job_drain = crate::jobs::RemoteJobDrain::from_resources(job_resources)
        .expect("remote recording drain must initialize before commands are registered");
    let builder = builder
        .manage(media_protocol::MediaOwner::new())
        .manage(crate::live::hotkey_commands::HotkeyEnrollmentGate::default())
        .manage(history::HistoryCatalogOwner::open_default())
        .manage(recording_jobs)
        .manage(remote_job_drain)
        .manage(crate::server_connector::ServerConnector::new());
    builder.invoke_handler(tauri::generate_handler![
        setup::setup_status,
        history::history_catalog,
        history::history_hide_native,
        history::history_migrate_hidden_paths,
        crate::server_connector::server_connection_status,
        crate::server_connector::refresh_server_connection,
        crate::server_connector::server_settings,
        crate::server_connector::set_server_settings,
        crate::jobs::commands::recording_jobs_snapshot,
        crate::jobs::commands::recording_jobs_pick_imports,
        crate::jobs::commands::recording_job_cancel,
        crate::jobs::commands::recording_job_dismiss,
        crate::jobs::commands::recording_job_retry,
        setup::fallback_model_status,
        setup::fallback_model_install,
        setup::fallback_model_cancel_install,
        setup::fallback_model_verify,
        setup::fallback_model_remove,
        setup::fallback_model_set_enabled,
        setup::fallback_model_open_folder,
        setup::list_local_compute_targets,
        setup::set_local_compute_target,
        live::live_status,
        live::live_overlay_status,
        live::show_live_overlay,
        live::hide_live_overlay,
        live::set_live_overlay_surface,
        live::set_live_overlay_enabled,
        crate::live::hotkey_commands::record_live_hotkey,
        crate::live::hotkey_commands::clear_live_hotkey,
        crate::live::hotkey_commands::reset_live_hotkey,
        crate::live::hotkey_commands::record_live_paste_hotkey,
        crate::live::hotkey_commands::clear_live_paste_hotkey,
        crate::live::hotkey_commands::reset_live_paste_hotkey,
        live::set_live_capture_mode,
        live::list_input_devices,
        live::set_input_device,
        live::preflight_input_device,
        live::start_live_session,
        live::start_live_overlay_session,
        live::stop_live_session,
        live::stop_live_overlay_session,
        live::recover_live_session,
        live::delete_recoverable_live_session,
        live::delete_saved_live_session,
        live::show_main_workspace,
        setup::polish_num_gpu,
        crate::file_actions::restore_recording_playback_path,
        crate::file_actions::release_recording_playback,
        crate::file_actions::resolve_owned_live_transcript_paths,
        crate::file_actions::read_text_file,
        crate::file_actions::read_text_preview,
        crate::file_actions::write_polished_text,
        crate::file_actions::open_app_path,
        crate::file_actions::reveal_app_path,
        #[cfg(feature = "wdio")]
        crate::tray::wdio_dispatch_tray_action
    ])
}
