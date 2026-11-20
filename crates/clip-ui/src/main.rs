mod gtk_ffi;

use std::cell::RefCell;
use std::ffi::{c_int, c_uint, c_void};
use std::path::PathBuf;
use std::ptr;
use std::rc::{Rc, Weak};

use clip_core::emoji::{
    emojis_for_category, load_recent_emojis, record_recent_emoji, search_emojis, EmojiCategory,
    EmojiEntry,
};
use clip_core::ipc::{send_request, Request, Response};
use clip_core::markdown::{is_markdown, markdown_to_pango_markup};
use clip_core::model::{format_byte_size, format_relative_time, now_millis, ClipboardEntry, EntryKind};
use clip_core::paths::default_socket_path;
use clip_core::search::highlight_pango_preview;
use clip_core::settings::Settings;

use crate::gtk_ffi::*;

const MAX_DISPLAY_ITEMS: u32 = 100;
const EMOJI_GRID_COLS: i32 = 8;
const MAX_EMOJI_SEARCH_RESULTS: usize = 160;

const APP_CSS: &str = "
.clip-search {
    font-size: 1.02em;
}
.clip-list {
    background: transparent;
}
.clip-list > row {
    border-radius: 10px;
    margin: 3px 0px;
    padding: 4px 6px;
    background-color: alpha(@card_bg_color, 0.72);
    border: 1px solid alpha(@borders, 0.55);
    transition: background-color 120ms ease;
}
.clip-list > row:hover {
    background-color: @card_bg_color;
}
.clip-list > row:selected {
    background-color: alpha(@accent_bg_color, 0.16);
    border-color: alpha(@accent_color, 0.55);
    color: @window_fg_color;
}
.clip-list > row:selected label {
    color: @window_fg_color;
}
.clip-pin-active {
    color: @accent_color;
}
.clip-thumb {
    border-radius: 6px;
    border: 1px solid alpha(@borders, 0.7);
    background-color: alpha(@window_fg_color, 0.04);
}
.clip-badge {
    font-size: 0.78em;
    font-weight: 600;
    padding: 1px 7px;
    border-radius: 99px;
    background-color: alpha(@window_fg_color, 0.08);
}
.clip-footer {
    font-size: 0.82em;
    opacity: 0.65;
}
.clip-settings {
    border-radius: 10px;
    padding: 8px 12px;
    background-color: @card_bg_color;
    border: 1px solid alpha(@borders, 0.7);
}
.clip-settings-btn-active {
    background-color: alpha(@accent_bg_color, 0.16);
    color: @accent_color;
}
.clip-tab-btn {
    padding: 4px 10px;
}
.clip-emoji-cat-btn {
    min-width: 34px;
    min-height: 30px;
    padding: 2px 4px;
    border-radius: 8px;
    font-size: 16px;
}
.clip-emoji-cat-active {
    background-color: alpha(@accent_bg_color, 0.22);
    color: @accent_color;
}
.clip-emoji-grid > flowboxchild {
    border-radius: 8px;
    padding: 4px;
    transition: background-color 100ms ease;
}
.clip-emoji-grid > flowboxchild:hover {
    background-color: alpha(@card_bg_color, 0.9);
}
.clip-emoji-grid > flowboxchild:selected {
    background-color: alpha(@accent_bg_color, 0.22);
    outline: 1px solid alpha(@accent_color, 0.55);
}
.clip-emoji-glyph {
    font-size: 22px;
}
.clip-emoji-info {
    font-size: 0.85em;
    opacity: 0.72;
}
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveTab {
    Clipboard,
    Emojis,
}

enum PendingAction {
    SelectEntry { id: i64, auto_paste: bool },
    PasteEmoji { emoji: String, auto_paste: bool },
}

struct CategoryClickData {
    state: Weak<RefCell<UiState>>,
    category: EmojiCategory,
}

struct UiState {
    socket_path: PathBuf,
    settings: Settings,
    window: *mut c_void,
    search_entry: *mut c_void,
    list_box: *mut c_void,
    scrolled: *mut c_void,
    stack: *mut c_void,
    status_page: *mut c_void,
    count_label: *mut c_void,
    clear_btn: *mut c_void,
    settings_revealer: *mut c_void,
    settings_btn: *mut c_void,
    tab_clipboard_btn: *mut c_void,
    tab_emoji_btn: *mut c_void,
    main_stack: *mut c_void,
    emoji_stack: *mut c_void,
    emoji_scrolled: *mut c_void,
    emoji_flow: *mut c_void,
    emoji_empty: *mut c_void,
    emoji_cat_btns: Vec<*mut c_void>,
    emoji_info_label: *mut c_void,
    footer_label: *mut c_void,
    active_tab: ActiveTab,
    active_emoji_cat: EmojiCategory,
    recent_emojis: Vec<String>,
    visible_emojis: Vec<&'static EmojiEntry>,
    selected_emoji_idx: i32,
    emoji_loaded: bool,
    has_been_active: bool,
    closing: bool,
    pending_action: Option<PendingAction>,
    entries: Vec<ClipboardEntry>,
}

struct RowActionData {
    state: Weak<RefCell<UiState>>,
    id: i64,
}

fn preview_text(raw: &str) -> String {
    let trimmed = raw.trim();
    let mut out = String::with_capacity(trimmed.len().min(260));
    let mut prev_space = false;
    let mut count = 0usize;
    for ch in trimmed.chars() {
        if count >= 240 {
            out.push('…');
            break;
        }
        if ch == '\n' || ch == '\r' || ch == '\t' {
            if !prev_space {
                out.push(' ');
                prev_space = true;
                count += 1;
            }
        } else if ch == ' ' {
            if !prev_space {
                out.push(' ');
                prev_space = true;
                count += 1;
            }
        } else {
            out.push(ch);
            prev_space = false;
            count += 1;
        }
    }
    out
}

unsafe fn install_css() {
    let display = gdk_display_get_default();
    if display.is_null() {
        return;
    }
    let provider = gtk_css_provider_new();
    let css = cstr(APP_CSS);
    gtk_css_provider_load_from_string(provider, css.as_ptr());
    gtk_style_context_add_provider_for_display(
        display,
        provider,
        GTK_STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    g_object_unref(provider);
}

unsafe fn set_empty_state(state: &UiState, icon: &str, title: &str, desc: &str) {
    let c_icon = cstr(icon);
    let c_title = cstr(title);
    let c_desc = cstr(desc);
    adw_status_page_set_icon_name(state.status_page, c_icon.as_ptr());
    adw_status_page_set_title(state.status_page, c_title.as_ptr());
    adw_status_page_set_description(state.status_page, c_desc.as_ptr());
    gtk_stack_set_visible_child_name(state.stack, c"empty".as_ptr());
}

unsafe fn build_row_widget(
    state_rc: &Rc<RefCell<UiState>>,
    entry: &ClipboardEntry,
    now_ms: i64,
    render_markdown: bool,
    query: &str,
) -> *mut c_void {
    let row = gtk_list_box_row_new();
    let hbox = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 10);
    set_margins(hbox, 6, 6, 8, 6);

    let is_md = render_markdown && entry.kind == EntryKind::Text && is_markdown(&entry.text);

    // Leading visual: thumbnail for images, symbolic icon for text/markdown.
    match entry.kind {
        EntryKind::Image => {
            let mut thumb_widget = ptr::null_mut();
            if let Some(ref png) = entry.image_png {
                let texture = make_texture_from_png(png);
                if !texture.is_null() {
                    let pic = gtk_picture_new_for_paintable(texture);
                    g_object_unref(texture);
                    gtk_picture_set_content_fit(pic, GTK_CONTENT_FIT_COVER);
                    gtk_widget_set_size_request(pic, 68, 48);
                    gtk_widget_set_overflow(pic, GTK_OVERFLOW_HIDDEN);
                    gtk_widget_add_css_class(pic, c"clip-thumb".as_ptr());
                    thumb_widget = pic;
                }
            }
            if thumb_widget.is_null() {
                let icon = gtk_image_new_from_icon_name(c"image-x-generic-symbolic".as_ptr());
                gtk_image_set_pixel_size(icon, 24);
                gtk_widget_set_size_request(icon, 36, 36);
                thumb_widget = icon;
            }
            gtk_widget_set_valign(thumb_widget, GTK_ALIGN_CENTER);
            gtk_box_append(hbox, thumb_widget);
        }
        EntryKind::Text => {
            let icon_name = if entry.pinned {
                c"view-pin-symbolic".as_ptr()
            } else if is_md {
                c"text-x-generic-symbolic".as_ptr()
            } else {
                c"edit-copy-symbolic".as_ptr()
            };
            let icon = gtk_image_new_from_icon_name(icon_name);
            gtk_image_set_pixel_size(icon, 16);
            gtk_widget_set_valign(icon, GTK_ALIGN_CENTER);
            if entry.pinned {
                gtk_widget_add_css_class(icon, c"clip-pin-active".as_ptr());
            } else {
                gtk_widget_add_css_class(icon, c"dim-label".as_ptr());
            }
            gtk_box_append(hbox, icon);
        }
    }

    // Center text column (title + metadata subtitle).
    let vbox = gtk_box_new(GTK_ORIENTATION_VERTICAL, 3);
    gtk_widget_set_hexpand(vbox, 1);
    gtk_widget_set_valign(vbox, GTK_ALIGN_CENTER);

    let title_lbl = if is_md {
        let markup = markdown_to_pango_markup(&entry.text, 3, 240);
        let c_markup = cstr(&markup);
        let lbl = gtk_label_new(ptr::null());
        gtk_label_set_markup(lbl, c_markup.as_ptr());
        gtk_label_set_lines(lbl, 3);
        lbl
    } else if entry.kind == EntryKind::Text && !query.trim().is_empty() {
        let markup = highlight_pango_preview(&preview_text(&entry.text), query);
        let c_markup = cstr(&markup);
        let lbl = gtk_label_new(ptr::null());
        gtk_label_set_markup(lbl, c_markup.as_ptr());
        gtk_label_set_lines(lbl, 2);
        lbl
    } else {
        let title_str = match entry.kind {
            EntryKind::Text => preview_text(&entry.text),
            EntryKind::Image => entry.text.clone(),
        };
        let c_title = cstr(&title_str);
        let lbl = gtk_label_new(c_title.as_ptr());
        gtk_label_set_lines(lbl, 2);
        lbl
    };
    gtk_label_set_xalign(title_lbl, 0.0);
    gtk_label_set_wrap(title_lbl, 1);
    gtk_label_set_ellipsize(title_lbl, PANGO_ELLIPSIZE_END);
    gtk_box_append(vbox, title_lbl);

    let rel = format_relative_time(entry.updated_at, now_ms);
    let meta = match entry.kind {
        EntryKind::Text => {
            let chars = entry.text.chars().count();
            match (entry.pinned, is_md) {
                (true, true) => format!("Pinned · Markdown · {} · {} chars", rel, chars),
                (false, true) => format!("Markdown · {} · {} chars", rel, chars),
                (true, false) => format!("Pinned · {} · {} chars", rel, chars),
                (false, false) => format!("{} · {} chars", rel, chars),
            }
        }
        EntryKind::Image => {
            let sz = format_byte_size(entry.byte_size);
            if entry.pinned {
                format!("Pinned · {} · {}", rel, sz)
            } else {
                format!("{} · {}", rel, sz)
            }
        }
    };
    let c_meta = cstr(&meta);
    let meta_lbl = gtk_label_new(c_meta.as_ptr());
    gtk_label_set_xalign(meta_lbl, 0.0);
    gtk_widget_add_css_class(meta_lbl, c"caption".as_ptr());
    gtk_widget_add_css_class(meta_lbl, c"dim-label".as_ptr());
    gtk_box_append(vbox, meta_lbl);

    gtk_box_append(hbox, vbox);

    // Trailing Pin & Delete icon buttons.
    let actions = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 2);
    gtk_widget_set_valign(actions, GTK_ALIGN_CENTER);

    let pin_btn = gtk_button_new_from_icon_name(c"view-pin-symbolic".as_ptr());
    gtk_widget_add_css_class(pin_btn, c"flat".as_ptr());
    gtk_widget_add_css_class(pin_btn, c"circular".as_ptr());
    if entry.pinned {
        gtk_widget_add_css_class(pin_btn, c"clip-pin-active".as_ptr());
        gtk_widget_set_tooltip_text(pin_btn, c"Unpin (Ctrl+P)".as_ptr());
    } else {
        gtk_widget_add_css_class(pin_btn, c"dim-label".as_ptr());
        gtk_widget_set_tooltip_text(pin_btn, c"Pin (Ctrl+P)".as_ptr());
    }
    gtk_widget_set_focusable(pin_btn, 0);
    connect_boxed(
        pin_btn,
        c"clicked",
        on_pin_clicked as GCallback,
        RowActionData {
            state: Rc::downgrade(state_rc),
            id: entry.id,
        },
    );
    gtk_box_append(actions, pin_btn);

    let del_btn = gtk_button_new_from_icon_name(c"edit-delete-symbolic".as_ptr());
    gtk_widget_add_css_class(del_btn, c"flat".as_ptr());
    gtk_widget_add_css_class(del_btn, c"circular".as_ptr());
    gtk_widget_add_css_class(del_btn, c"dim-label".as_ptr());
    gtk_widget_set_tooltip_text(del_btn, c"Remove (Del)".as_ptr());
    gtk_widget_set_focusable(del_btn, 0);
    connect_boxed(
        del_btn,
        c"clicked",
        on_delete_clicked as GCallback,
        RowActionData {
            state: Rc::downgrade(state_rc),
            id: entry.id,
        },
    );
    gtk_box_append(actions, del_btn);

    gtk_box_append(hbox, actions);
    gtk_list_box_row_set_child(row, hbox);
    row
}

unsafe fn refresh_list(state_rc: &Rc<RefCell<UiState>>) {
    let (socket_path, query) = {
        let s = state_rc.borrow();
        let q = ptr_to_string(gtk_editable_get_text(s.search_entry));
        (s.socket_path.clone(), q)
    };

    let req = if query.trim().is_empty() {
        Request::List {
            limit: MAX_DISPLAY_ITEMS,
            include_images: true,
        }
    } else {
        Request::Search {
            query: query.clone(),
            limit: MAX_DISPLAY_ITEMS,
            include_images: true,
        }
    };

    let resp = send_request(&socket_path, &req);
    let mut s = state_rc.borrow_mut();
    gtk_list_box_remove_all(s.list_box);
    s.entries.clear();

    match resp {
        Ok(Response::Entries(entries)) => {
            let count_txt = cstr(&format!("{} items", entries.len()));
            gtk_label_set_text(s.count_label, count_txt.as_ptr());

            if entries.is_empty() {
                if query.trim().is_empty() {
                    set_empty_state(
                        &s,
                        "edit-paste-symbolic",
                        "Clipboard History Empty",
                        "Copy text or an image anywhere, then press Super+V.",
                    );
                } else {
                    set_empty_state(
                        &s,
                        "edit-find-symbolic",
                        "No Matches",
                        "Try another search query.",
                    );
                }
                return;
            }

            let now_ms = now_millis();
            let render_md = s.settings.render_markdown;
            for entry in &entries {
                let row = build_row_widget(state_rc, entry, now_ms, render_md, &query);
                gtk_list_box_append(s.list_box, row);
            }
            s.entries = entries;

            gtk_stack_set_visible_child_name(s.stack, c"list".as_ptr());
            let first = gtk_list_box_get_row_at_index(s.list_box, 0);
            if !first.is_null() {
                gtk_list_box_select_row(s.list_box, first);
            }
        }
        Ok(Response::Error(msg)) => {
            let count_txt = cstr("0 items");
            gtk_label_set_text(s.count_label, count_txt.as_ptr());
            set_empty_state(&s, "dialog-warning-symbolic", "Daemon Error", &msg);
        }
        Ok(_) => {}
        Err(err) => {
            let count_txt = cstr("offline");
            gtk_label_set_text(s.count_label, count_txt.as_ptr());
            set_empty_state(
                &s,
                "network-offline-symbolic",
                "Clipboard Daemon Not Running",
                &err,
            );
        }
    }
}

unsafe fn trigger_close_with_action(state_rc: &Rc<RefCell<UiState>>, action: PendingAction) {
    let window = {
        let mut s = state_rc.borrow_mut();
        s.closing = true;
        s.pending_action = Some(action);
        s.window
    };
    if !window.is_null() {
        gtk_widget_set_visible(window, 0);
        gtk_window_close(window);
    }
}

unsafe fn select_index(state_rc: &Rc<RefCell<UiState>>, index: i32) {
    let action_opt = {
        let s = state_rc.borrow();
        usize::try_from(index)
            .ok()
            .and_then(|i| s.entries.get(i))
            .map(|e| PendingAction::SelectEntry {
                id: e.id,
                auto_paste: s.settings.auto_paste,
            })
    };
    if let Some(action) = action_opt {
        trigger_close_with_action(state_rc, action);
    }
}

unsafe fn current_selected_index(s: &UiState) -> Option<i32> {
    let row = gtk_list_box_get_selected_row(s.list_box);
    if row.is_null() {
        None
    } else {
        let idx = gtk_list_box_row_get_index(row);
        if idx >= 0 {
            Some(idx)
        } else {
            None
        }
    }
}

unsafe fn scroll_child_into_view(
    scrolled: *mut c_void,
    child: *mut c_void,
    container: *mut c_void,
    fallback_top: f64,
    fallback_height: f64,
) {
    let vadj = gtk_scrolled_window_get_vadjustment(scrolled);
    if vadj.is_null() {
        return;
    }
    let page = gtk_adjustment_get_page_size(vadj);
    let cur_scroll = gtk_adjustment_get_value(vadj);
    let mut bounds = GrapheneRect::default();
    let (top, bottom) = if gtk_widget_compute_bounds(child, container, &mut bounds) != 0 {
        (bounds.y as f64, (bounds.y + bounds.height) as f64)
    } else {
        (fallback_top, fallback_top + fallback_height)
    };
    if top < cur_scroll {
        gtk_adjustment_set_value(vadj, top);
    } else if bottom > cur_scroll + page {
        gtk_adjustment_set_value(vadj, bottom - page);
    }
}

unsafe fn move_selection(s: &UiState, delta: i32) {
    let len = s.entries.len() as i32;
    if len <= 0 {
        return;
    }
    let cur = current_selected_index(s).unwrap_or(0);
    let next = (cur + delta).clamp(0, len - 1);
    let row = gtk_list_box_get_row_at_index(s.list_box, next);
    if !row.is_null() {
        gtk_list_box_select_row(s.list_box, row);
        scroll_child_into_view(s.scrolled, row, s.list_box, (next as f64) * 66.0, 66.0);
    }
}

unsafe fn update_emoji_info_label(s: &UiState, is_search: bool) {
    let prefix = if is_search {
        "Search"
    } else {
        s.active_emoji_cat.label()
    };
    let selected = usize::try_from(s.selected_emoji_idx)
        .ok()
        .and_then(|i| s.visible_emojis.get(i));
    let txt = match selected {
        Some(e) => format!("{} · {} {}", prefix, e.glyph, e.name),
        None => prefix.to_string(),
    };
    let c_txt = cstr(&txt);
    gtk_label_set_text(s.emoji_info_label, c_txt.as_ptr());
}

unsafe fn update_emoji_category_buttons(s: &UiState, is_search: bool) {
    for (idx, &btn) in s.emoji_cat_btns.iter().enumerate() {
        if btn.is_null() {
            continue;
        }
        gtk_widget_remove_css_class(btn, c"clip-emoji-cat-active".as_ptr());
        if !is_search && EmojiCategory::ALL.get(idx) == Some(&s.active_emoji_cat) {
            gtk_widget_add_css_class(btn, c"clip-emoji-cat-active".as_ptr());
        }
    }
}

unsafe fn refresh_emojis(state_rc: &Rc<RefCell<UiState>>) {
    let need_load = !state_rc.borrow().emoji_loaded;
    if need_load {
        let loaded = load_recent_emojis();
        let mut s = state_rc.borrow_mut();
        s.recent_emojis = loaded;
        s.emoji_loaded = true;
    }

    let query = {
        let s = state_rc.borrow();
        ptr_to_string(gtk_editable_get_text(s.search_entry))
    };
    let is_search = !query.trim().is_empty();

    let mut s = state_rc.borrow_mut();
    let items = if is_search {
        search_emojis(&query, &s.recent_emojis, MAX_EMOJI_SEARCH_RESULTS)
    } else {
        emojis_for_category(s.active_emoji_cat, &s.recent_emojis)
    };

    let count_str = if is_search {
        format!("{} matches", items.len())
    } else {
        format!("{} emojis", items.len())
    };
    let c_count = cstr(&count_str);
    gtk_label_set_text(s.count_label, c_count.as_ptr());

    gtk_flow_box_remove_all(s.emoji_flow);
    s.visible_emojis = items;
    update_emoji_category_buttons(&s, is_search);

    if s.visible_emojis.is_empty() {
        s.selected_emoji_idx = 0;
        gtk_stack_set_visible_child_name(s.emoji_stack, c"empty".as_ptr());
        update_emoji_info_label(&s, is_search);
        return;
    }

    for entry in &s.visible_emojis {
        let child = gtk_flow_box_child_new();
        gtk_widget_set_focusable(child, 0);
        let c_tip = cstr(entry.name);
        gtk_widget_set_tooltip_text(child, c_tip.as_ptr());

        let c_glyph = cstr(entry.glyph);
        let lbl = gtk_label_new(c_glyph.as_ptr());
        gtk_widget_add_css_class(lbl, c"clip-emoji-glyph".as_ptr());
        gtk_flow_box_child_set_child(child, lbl);
        gtk_flow_box_append(s.emoji_flow, child);
    }

    gtk_stack_set_visible_child_name(s.emoji_stack, c"grid".as_ptr());
    s.selected_emoji_idx = 0;
    let first = gtk_flow_box_get_child_at_index(s.emoji_flow, 0);
    if !first.is_null() {
        gtk_flow_box_select_child(s.emoji_flow, first);
    }
    let vadj = gtk_scrolled_window_get_vadjustment(s.emoji_scrolled);
    if !vadj.is_null() {
        gtk_adjustment_set_value(vadj, 0.0);
    }
    update_emoji_info_label(&s, is_search);
}

unsafe fn move_emoji_selection(s: &mut UiState, delta: i32) {
    let len = s.visible_emojis.len() as i32;
    if len <= 0 {
        return;
    }
    let cur = s.selected_emoji_idx.clamp(0, len - 1);
    let next = (cur + delta).clamp(0, len - 1);
    s.selected_emoji_idx = next;

    let child = gtk_flow_box_get_child_at_index(s.emoji_flow, next);
    if !child.is_null() {
        gtk_flow_box_select_child(s.emoji_flow, child);
        let row_est = ((next / EMOJI_GRID_COLS) as f64) * 44.0;
        scroll_child_into_view(s.emoji_scrolled, child, s.emoji_flow, row_est, 44.0);
    }
    let is_search = !ptr_to_string(gtk_editable_get_text(s.search_entry))
        .trim()
        .is_empty();
    update_emoji_info_label(s, is_search);
}

unsafe fn select_emoji_index(state_rc: &Rc<RefCell<UiState>>, index: i32) {
    let glyph_and_paste = {
        let s = state_rc.borrow();
        usize::try_from(index)
            .ok()
            .and_then(|i| s.visible_emojis.get(i))
            .map(|e| (e.glyph.to_string(), s.settings.auto_paste))
    };
    if let Some((glyph, auto_paste)) = glyph_and_paste {
        if let Err(err) = record_recent_emoji(&glyph) {
            eprintln!("clip-ui: failed to save recent emoji: {}", err);
        }
        {
            let mut s = state_rc.borrow_mut();
            s.recent_emojis.retain(|item| item != &glyph);
            s.recent_emojis.insert(0, glyph.clone());
        }
        trigger_close_with_action(
            state_rc,
            PendingAction::PasteEmoji {
                emoji: glyph,
                auto_paste,
            },
        );
    }
}

unsafe fn apply_tab_chrome(
    state_rc: &Rc<RefCell<UiState>>,
    tab: ActiveTab,
    clear_btn: *mut c_void,
    search_entry: *mut c_void,
    main_stack: *mut c_void,
    footer_label: *mut c_void,
    clip_btn: *mut c_void,
    emoji_btn: *mut c_void,
) {
    match tab {
        ActiveTab::Clipboard => {
            if gtk_toggle_button_get_active(clip_btn) == 0 {
                gtk_toggle_button_set_active(clip_btn, 1);
            }
            gtk_widget_set_visible(clear_btn, 1);
            gtk_search_entry_set_placeholder_text(
                search_entry,
                c"Search text, images, or filters (img:, md:, pin:, url:)…".as_ptr(),
            );
            gtk_label_set_text(
                footer_label,
                c"↑↓ Navigate · Enter Paste · Ctrl+P Pin · Del Remove · Super+. Emojis · Esc Close"
                    .as_ptr(),
            );
            gtk_stack_set_visible_child_name(main_stack, c"clipboard".as_ptr());
            refresh_list(state_rc);
        }
        ActiveTab::Emojis => {
            if gtk_toggle_button_get_active(emoji_btn) == 0 {
                gtk_toggle_button_set_active(emoji_btn, 1);
            }
            gtk_widget_set_visible(clear_btn, 0);
            gtk_search_entry_set_placeholder_text(
                search_entry,
                c"Search emojis (English, Arabic, or :), <3)…".as_ptr(),
            );
            gtk_label_set_text(
                footer_label,
                c"Arrows Navigate · Enter Paste · Super+V Clipboard · Esc Close".as_ptr(),
            );
            gtk_stack_set_visible_child_name(main_stack, c"emojis".as_ptr());
            refresh_emojis(state_rc);
        }
    }
    gtk_widget_grab_focus(search_entry);
}

unsafe fn switch_tab(state_rc: &Rc<RefCell<UiState>>, tab: ActiveTab) {
    let (clear_btn, search_entry, main_stack, footer_label, clip_btn, emoji_btn) = {
        let mut s = state_rc.borrow_mut();
        if s.active_tab == tab {
            return;
        }
        s.active_tab = tab;
        (
            s.clear_btn,
            s.search_entry,
            s.main_stack,
            s.footer_label,
            s.tab_clipboard_btn,
            s.tab_emoji_btn,
        )
    };

    apply_tab_chrome(
        state_rc,
        tab,
        clear_btn,
        search_entry,
        main_stack,
        footer_label,
        clip_btn,
        emoji_btn,
    );
}

unsafe extern "C" fn on_search_changed(_entry: *mut c_void, user_data: *mut c_void) {
    let weak = &*(user_data as *const Weak<RefCell<UiState>>);
    if let Some(state_rc) = weak.upgrade() {
        let tab = state_rc.borrow().active_tab;
        match tab {
            ActiveTab::Clipboard => refresh_list(&state_rc),
            ActiveTab::Emojis => refresh_emojis(&state_rc),
        }
    }
}

unsafe extern "C" fn on_tab_clipboard_toggled(btn: *mut c_void, user_data: *mut c_void) {
    if gtk_toggle_button_get_active(btn) == 0 {
        return;
    }
    let weak = &*(user_data as *const Weak<RefCell<UiState>>);
    if let Some(state_rc) = weak.upgrade() {
        switch_tab(&state_rc, ActiveTab::Clipboard);
    }
}

unsafe extern "C" fn on_tab_emojis_toggled(btn: *mut c_void, user_data: *mut c_void) {
    if gtk_toggle_button_get_active(btn) == 0 {
        return;
    }
    let weak = &*(user_data as *const Weak<RefCell<UiState>>);
    if let Some(state_rc) = weak.upgrade() {
        switch_tab(&state_rc, ActiveTab::Emojis);
    }
}

unsafe extern "C" fn on_emoji_category_clicked(_btn: *mut c_void, user_data: *mut c_void) {
    let data = &*(user_data as *const CategoryClickData);
    let Some(state_rc) = data.state.upgrade() else {
        return;
    };
    let search_entry = {
        let mut s = state_rc.borrow_mut();
        s.active_emoji_cat = data.category;
        s.search_entry
    };
    let current = ptr_to_string(gtk_editable_get_text(search_entry));
    if current.is_empty() {
        refresh_emojis(&state_rc);
    } else {
        // Clearing triggers on_search_changed -> refresh_emojis.
        gtk_editable_set_text(search_entry, c"".as_ptr());
    }
    gtk_widget_grab_focus(search_entry);
}

unsafe extern "C" fn on_emoji_child_activated(
    _flow_box: *mut c_void,
    child: *mut c_void,
    user_data: *mut c_void,
) {
    let weak = &*(user_data as *const Weak<RefCell<UiState>>);
    if let Some(state_rc) = weak.upgrade() {
        let idx = gtk_flow_box_child_get_index(child);
        if idx >= 0 {
            select_emoji_index(&state_rc, idx);
        }
    }
}

unsafe extern "C" fn on_row_activated(
    _list_box: *mut c_void,
    row: *mut c_void,
    user_data: *mut c_void,
) {
    let weak = &*(user_data as *const Weak<RefCell<UiState>>);
    if let Some(state_rc) = weak.upgrade() {
        let idx = gtk_list_box_row_get_index(row);
        if idx >= 0 {
            select_index(&state_rc, idx);
        }
    }
}

unsafe extern "C" fn on_pin_clicked(_btn: *mut c_void, user_data: *mut c_void) {
    let data = &*(user_data as *const RowActionData);
    if let Some(state_rc) = data.state.upgrade() {
        let socket_path = state_rc.borrow().socket_path.clone();
        let _ = send_request(&socket_path, &Request::TogglePin { id: data.id });
        refresh_list(&state_rc);
        let search_entry = state_rc.borrow().search_entry;
        gtk_widget_grab_focus(search_entry);
    }
}

unsafe extern "C" fn on_delete_clicked(_btn: *mut c_void, user_data: *mut c_void) {
    let data = &*(user_data as *const RowActionData);
    if let Some(state_rc) = data.state.upgrade() {
        let socket_path = state_rc.borrow().socket_path.clone();
        let _ = send_request(&socket_path, &Request::Delete { id: data.id });
        refresh_list(&state_rc);
        let search_entry = state_rc.borrow().search_entry;
        gtk_widget_grab_focus(search_entry);
    }
}

unsafe extern "C" fn on_clear_clicked(_btn: *mut c_void, user_data: *mut c_void) {
    let weak = &*(user_data as *const Weak<RefCell<UiState>>);
    if let Some(state_rc) = weak.upgrade() {
        let socket_path = state_rc.borrow().socket_path.clone();
        let _ = send_request(&socket_path, &Request::Clear);
        refresh_list(&state_rc);
        let search_entry = state_rc.borrow().search_entry;
        gtk_widget_grab_focus(search_entry);
    }
}

unsafe extern "C" fn on_key_pressed(
    _controller: *mut c_void,
    keyval: c_uint,
    _keycode: c_uint,
    state_mask: c_uint,
    user_data: *mut c_void,
) -> c_int {
    let weak = &*(user_data as *const Weak<RefCell<UiState>>);
    let Some(state_rc) = weak.upgrade() else {
        return 0;
    };

    let ctrl = (state_mask & GDK_CONTROL_MASK) != 0;
    let alt = (state_mask & GDK_ALT_MASK) != 0;

    if (ctrl && (keyval == GDK_KEY_TAB || keyval == GDK_KEY_ISO_LEFT_TAB))
        || (alt && (keyval == GDK_KEY_1 || keyval == GDK_KEY_2))
    {
        let target = if alt {
            if keyval == GDK_KEY_1 {
                ActiveTab::Clipboard
            } else {
                ActiveTab::Emojis
            }
        } else {
            match state_rc.borrow().active_tab {
                ActiveTab::Clipboard => ActiveTab::Emojis,
                ActiveTab::Emojis => ActiveTab::Clipboard,
            }
        };
        switch_tab(&state_rc, target);
        return 1;
    }

    if keyval == GDK_KEY_ESCAPE {
        let (search_entry, window) = {
            let s = state_rc.borrow();
            (s.search_entry, s.window)
        };
        let current = ptr_to_string(gtk_editable_get_text(search_entry));
        if !current.is_empty() {
            gtk_editable_set_text(search_entry, c"".as_ptr());
            gtk_widget_grab_focus(search_entry);
        } else if !window.is_null() {
            gtk_window_close(window);
        }
        return 1;
    }

    let active_tab = state_rc.borrow().active_tab;
    if active_tab == ActiveTab::Emojis {
        match keyval {
            GDK_KEY_DOWN => {
                let mut s = state_rc.borrow_mut();
                move_emoji_selection(&mut s, EMOJI_GRID_COLS);
                1
            }
            GDK_KEY_UP => {
                let mut s = state_rc.borrow_mut();
                move_emoji_selection(&mut s, -EMOJI_GRID_COLS);
                1
            }
            GDK_KEY_RIGHT | GDK_KEY_LEFT => {
                let mut s = state_rc.borrow_mut();
                let search_empty = ptr_to_string(gtk_editable_get_text(s.search_entry)).is_empty();
                if search_empty {
                    let delta = if keyval == GDK_KEY_RIGHT { 1 } else { -1 };
                    move_emoji_selection(&mut s, delta);
                    1
                } else {
                    0
                }
            }
            GDK_KEY_RETURN | GDK_KEY_KP_ENTER => {
                let idx = state_rc.borrow().selected_emoji_idx;
                select_emoji_index(&state_rc, idx);
                1
            }
            _ => 0,
        }
    } else {
        match keyval {
            GDK_KEY_DOWN => {
                let s = state_rc.borrow();
                move_selection(&s, 1);
                1
            }
            GDK_KEY_UP => {
                let s = state_rc.borrow();
                move_selection(&s, -1);
                1
            }
            GDK_KEY_RETURN | GDK_KEY_KP_ENTER => {
                let idx = {
                    let s = state_rc.borrow();
                    current_selected_index(&s).unwrap_or(0)
                };
                select_index(&state_rc, idx);
                1
            }
            GDK_KEY_P_LOWER | GDK_KEY_P_UPPER if ctrl => {
                let (socket_path, id_opt) = {
                    let s = state_rc.borrow();
                    let id_opt = current_selected_index(&s)
                        .and_then(|i| usize::try_from(i).ok())
                        .and_then(|i| s.entries.get(i))
                        .map(|e| e.id);
                    (s.socket_path.clone(), id_opt)
                };
                if let Some(id) = id_opt {
                    let _ = send_request(&socket_path, &Request::TogglePin { id });
                    refresh_list(&state_rc);
                }
                1
            }
            GDK_KEY_DELETE => {
                let (socket_path, search_empty, id_opt) = {
                    let s = state_rc.borrow();
                    let txt = ptr_to_string(gtk_editable_get_text(s.search_entry));
                    let id_opt = current_selected_index(&s)
                        .and_then(|i| usize::try_from(i).ok())
                        .and_then(|i| s.entries.get(i))
                        .map(|e| e.id);
                    (s.socket_path.clone(), txt.is_empty(), id_opt)
                };
                if ctrl || search_empty {
                    if let Some(id) = id_opt {
                        let _ = send_request(&socket_path, &Request::Delete { id });
                        refresh_list(&state_rc);
                    }
                    return 1;
                }
                0
            }
            _ => 0,
        }
    }
}

unsafe extern "C" fn on_window_close_request(_win: *mut c_void, user_data: *mut c_void) -> c_int {
    let weak = &*(user_data as *const Weak<RefCell<UiState>>);
    if let Some(state_rc) = weak.upgrade() {
        let mut s = state_rc.borrow_mut();
        s.window = ptr::null_mut();
        s.search_entry = ptr::null_mut();
        s.list_box = ptr::null_mut();
        s.scrolled = ptr::null_mut();
        s.stack = ptr::null_mut();
        s.status_page = ptr::null_mut();
        s.count_label = ptr::null_mut();
        s.clear_btn = ptr::null_mut();
        s.settings_revealer = ptr::null_mut();
        s.settings_btn = ptr::null_mut();
        s.tab_clipboard_btn = ptr::null_mut();
        s.tab_emoji_btn = ptr::null_mut();
        s.main_stack = ptr::null_mut();
        s.emoji_stack = ptr::null_mut();
        s.emoji_scrolled = ptr::null_mut();
        s.emoji_flow = ptr::null_mut();
        s.emoji_empty = ptr::null_mut();
        s.emoji_info_label = ptr::null_mut();
        s.footer_label = ptr::null_mut();
        s.emoji_cat_btns.clear();
        s.visible_emojis.clear();
        s.has_been_active = false;
        s.closing = false;
    }
    0
}

unsafe extern "C" fn on_is_active_changed(
    win: *mut c_void,
    _pspec: *mut c_void,
    user_data: *mut c_void,
) {
    let weak = &*(user_data as *const Weak<RefCell<UiState>>);
    let Some(state_rc) = weak.upgrade() else {
        return;
    };
    let is_active = gtk_window_is_active(win) != 0;
    let should_close = {
        let mut s = state_rc.borrow_mut();
        if is_active {
            s.has_been_active = true;
            false
        } else if s.has_been_active && s.settings.close_on_focus_loss && !s.closing {
            s.closing = true;
            true
        } else {
            false
        }
    };
    if should_close {
        gtk_window_close(win);
    }
}

unsafe extern "C" fn on_settings_toggled(_btn: *mut c_void, user_data: *mut c_void) {
    let weak = &*(user_data as *const Weak<RefCell<UiState>>);
    let Some(state_rc) = weak.upgrade() else {
        return;
    };
    let revealer = state_rc.borrow().settings_revealer;
    if !revealer.is_null() {
        let revealed = gtk_revealer_get_reveal_child(revealer) != 0;
        gtk_revealer_set_reveal_child(revealer, if revealed { 0 } else { 1 });
    }
}

unsafe fn update_setting(
    switch: *mut c_void,
    user_data: *mut c_void,
    refresh: bool,
    apply: impl FnOnce(&mut Settings, bool),
) {
    let weak = &*(user_data as *const Weak<RefCell<UiState>>);
    let Some(state_rc) = weak.upgrade() else {
        return;
    };
    let active = gtk_switch_get_active(switch) != 0;
    {
        let mut s = state_rc.borrow_mut();
        apply(&mut s.settings, active);
        if let Err(e) = s.settings.save() {
            eprintln!("clip-ui: failed to save settings: {e}");
        }
    }
    if refresh {
        refresh_list(&state_rc);
    }
}

unsafe extern "C" fn on_auto_paste_toggled(
    switch: *mut c_void,
    _pspec: *mut c_void,
    user_data: *mut c_void,
) {
    update_setting(switch, user_data, false, |cfg, val| cfg.auto_paste = val);
}

unsafe extern "C" fn on_focus_close_toggled(
    switch: *mut c_void,
    _pspec: *mut c_void,
    user_data: *mut c_void,
) {
    update_setting(switch, user_data, false, |cfg, val| {
        cfg.close_on_focus_loss = val;
    });
}

unsafe extern "C" fn on_markdown_toggled(
    switch: *mut c_void,
    _pspec: *mut c_void,
    user_data: *mut c_void,
) {
    update_setting(switch, user_data, true, |cfg, val| cfg.render_markdown = val);
}

unsafe fn append_settings_row(
    parent: *mut c_void,
    title: &std::ffi::CStr,
    subtitle: &std::ffi::CStr,
    initial: bool,
    on_toggle: GCallback,
    state_rc: &Rc<RefCell<UiState>>,
) {
    let row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 12);
    gtk_widget_set_hexpand(row, 1);

    let text_box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 2);
    gtk_widget_set_hexpand(text_box, 1);
    gtk_widget_set_valign(text_box, GTK_ALIGN_CENTER);

    let title_lbl = gtk_label_new(title.as_ptr());
    gtk_label_set_xalign(title_lbl, 0.0);
    let sub_lbl = gtk_label_new(subtitle.as_ptr());
    gtk_label_set_xalign(sub_lbl, 0.0);
    gtk_widget_add_css_class(sub_lbl, c"dim-label".as_ptr());
    gtk_widget_add_css_class(sub_lbl, c"caption".as_ptr());
    gtk_box_append(text_box, title_lbl);
    gtk_box_append(text_box, sub_lbl);

    let switch = gtk_switch_new();
    gtk_widget_set_valign(switch, GTK_ALIGN_CENTER);
    gtk_widget_set_focusable(switch, 0);
    gtk_switch_set_active(switch, if initial { 1 } else { 0 });
    connect_boxed(switch, c"notify::active", on_toggle, Rc::downgrade(state_rc));

    gtk_box_append(row, text_box);
    gtk_box_append(row, switch);
    gtk_box_append(parent, row);
}

unsafe fn make_tab_button(
    icon_name: &std::ffi::CStr,
    label: &std::ffi::CStr,
    group_with: Option<*mut c_void>,
) -> *mut c_void {
    let btn = gtk_toggle_button_new();
    gtk_widget_set_focusable(btn, 0);
    gtk_widget_add_css_class(btn, c"clip-tab-btn".as_ptr());
    let content = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 6);
    let icon = gtk_image_new_from_icon_name(icon_name.as_ptr());
    gtk_image_set_pixel_size(icon, 15);
    let lbl = gtk_label_new(label.as_ptr());
    gtk_box_append(content, icon);
    gtk_box_append(content, lbl);
    gtk_button_set_child(btn, content);
    if let Some(leader) = group_with {
        gtk_toggle_button_set_group(btn, leader);
    }
    btn
}

unsafe fn open_window(
    app: *mut c_void,
    state_rc: &Rc<RefCell<UiState>>,
    initial_tab: ActiveTab,
) {
    let settings = Settings::load();
    {
        let mut s = state_rc.borrow_mut();
        s.settings = settings.clone();
        s.active_tab = initial_tab;
        s.has_been_active = false;
        s.closing = false;
        s.pending_action = None;
    }

    install_css();

    let window = adw_application_window_new(app);
    gtk_window_set_title(window, c"Clipboard".as_ptr());
    gtk_window_set_default_size(window, 480, 560);

    let root_box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);

    // Native GNOME/Ubuntu HeaderBar with linked Clipboard / Emojis tab switcher.
    let header = adw_header_bar_new();
    let tab_switcher = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 0);
    gtk_widget_add_css_class(tab_switcher, c"linked".as_ptr());
    let tab_clipboard_btn = make_tab_button(c"edit-paste-symbolic", c"Clipboard", None);
    let tab_emoji_btn = make_tab_button(
        c"face-smile-symbolic",
        c"Emojis",
        Some(tab_clipboard_btn),
    );
    match initial_tab {
        ActiveTab::Clipboard => gtk_toggle_button_set_active(tab_clipboard_btn, 1),
        ActiveTab::Emojis => gtk_toggle_button_set_active(tab_emoji_btn, 1),
    }
    gtk_box_append(tab_switcher, tab_clipboard_btn);
    gtk_box_append(tab_switcher, tab_emoji_btn);
    adw_header_bar_set_title_widget(header, tab_switcher);

    let count_lbl = gtk_label_new(c"".as_ptr());
    gtk_widget_add_css_class(count_lbl, c"clip-badge".as_ptr());
    gtk_widget_set_valign(count_lbl, GTK_ALIGN_CENTER);
    set_margins(count_lbl, 0, 0, 6, 0);
    adw_header_bar_pack_start(header, count_lbl);

    let clear_btn = gtk_button_new_from_icon_name(c"user-trash-symbolic".as_ptr());
    gtk_widget_add_css_class(clear_btn, c"flat".as_ptr());
    gtk_widget_set_tooltip_text(clear_btn, c"Clear unpinned history".as_ptr());
    gtk_widget_set_focusable(clear_btn, 0);
    adw_header_bar_pack_end(header, clear_btn);

    let settings_btn = gtk_button_new_from_icon_name(c"emblem-system-symbolic".as_ptr());
    gtk_widget_add_css_class(settings_btn, c"flat".as_ptr());
    gtk_widget_set_tooltip_text(settings_btn, c"Clipboard options".as_ptr());
    gtk_widget_set_focusable(settings_btn, 0);
    adw_header_bar_pack_end(header, settings_btn);

    gtk_box_append(root_box, header);

    // Collapsible inline options drawer (avoids popup focus-loss issues).
    let settings_revealer = gtk_revealer_new();
    let settings_box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 10);
    gtk_widget_add_css_class(settings_box, c"clip-settings".as_ptr());
    set_margins(settings_box, 8, 8, 12, 12);

    append_settings_row(
        settings_box,
        c"Auto-paste on select",
        c"Paste into the active field and close",
        settings.auto_paste,
        on_auto_paste_toggled as GCallback,
        state_rc,
    );
    append_settings_row(
        settings_box,
        c"Close on window switch",
        c"Hide automatically when focus leaves",
        settings.close_on_focus_loss,
        on_focus_close_toggled as GCallback,
        state_rc,
    );
    append_settings_row(
        settings_box,
        c"Format Markdown text",
        c"Render Markdown styling in previews",
        settings.render_markdown,
        on_markdown_toggled as GCallback,
        state_rc,
    );

    gtk_revealer_set_child(settings_revealer, settings_box);
    gtk_revealer_set_reveal_child(settings_revealer, 0);
    gtk_box_append(root_box, settings_revealer);

    // Search box shared across Clipboard and Emojis tabs.
    let search_entry = gtk_search_entry_new();
    gtk_search_entry_set_placeholder_text(search_entry, c"Search copied text or images…".as_ptr());
    gtk_widget_add_css_class(search_entry, c"clip-search".as_ptr());
    set_margins(search_entry, 8, 6, 12, 12);
    gtk_box_append(root_box, search_entry);

    // Outer stack switching between Clipboard and Emojis views.
    let main_stack = gtk_stack_new();
    gtk_widget_set_vexpand(main_stack, 1);
    gtk_widget_set_hexpand(main_stack, 1);

    // Clipboard page: List vs Empty/Offline StatusPage.
    let stack = gtk_stack_new();
    gtk_widget_set_vexpand(stack, 1);
    gtk_widget_set_hexpand(stack, 1);

    let scrolled = gtk_scrolled_window_new();
    gtk_scrolled_window_set_policy(scrolled, GTK_POLICY_NEVER, GTK_POLICY_AUTOMATIC);
    gtk_widget_set_vexpand(scrolled, 1);

    let list_box = gtk_list_box_new();
    gtk_list_box_set_selection_mode(list_box, GTK_SELECTION_SINGLE);
    gtk_list_box_set_activate_on_single_click(list_box, 1);
    gtk_widget_add_css_class(list_box, c"clip-list".as_ptr());
    set_margins(list_box, 4, 8, 12, 12);

    gtk_scrolled_window_set_child(scrolled, list_box);
    gtk_stack_add_named(stack, scrolled, c"list".as_ptr());

    let status_page = adw_status_page_new();
    gtk_stack_add_named(stack, status_page, c"empty".as_ptr());
    gtk_stack_add_named(main_stack, stack, c"clipboard".as_ptr());

    // Emojis page: category strip + status line + FlowBox grid / empty state.
    let emoji_page = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
    let cat_bar = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 2);
    gtk_widget_set_hexpand(cat_bar, 1);
    set_margins(cat_bar, 0, 4, 12, 12);

    let mut emoji_cat_btns = Vec::with_capacity(EmojiCategory::ALL.len());
    for &cat in &EmojiCategory::ALL {
        let c_icon = cstr(cat.icon());
        let btn = gtk_button_new_with_label(c_icon.as_ptr());
        gtk_widget_add_css_class(btn, c"flat".as_ptr());
        gtk_widget_add_css_class(btn, c"clip-emoji-cat-btn".as_ptr());
        gtk_widget_set_hexpand(btn, 1);
        gtk_widget_set_focusable(btn, 0);
        let c_label = cstr(cat.label());
        gtk_widget_set_tooltip_text(btn, c_label.as_ptr());
        connect_boxed(
            btn,
            c"clicked",
            on_emoji_category_clicked as GCallback,
            CategoryClickData {
                state: Rc::downgrade(state_rc),
                category: cat,
            },
        );
        gtk_box_append(cat_bar, btn);
        emoji_cat_btns.push(btn);
    }
    gtk_box_append(emoji_page, cat_bar);

    let emoji_info_label = gtk_label_new(c"".as_ptr());
    gtk_label_set_xalign(emoji_info_label, 0.0);
    gtk_widget_add_css_class(emoji_info_label, c"clip-emoji-info".as_ptr());
    set_margins(emoji_info_label, 0, 4, 14, 14);
    gtk_box_append(emoji_page, emoji_info_label);

    let emoji_stack = gtk_stack_new();
    gtk_widget_set_vexpand(emoji_stack, 1);
    gtk_widget_set_hexpand(emoji_stack, 1);

    let emoji_scrolled = gtk_scrolled_window_new();
    gtk_scrolled_window_set_policy(emoji_scrolled, GTK_POLICY_NEVER, GTK_POLICY_AUTOMATIC);
    gtk_widget_set_vexpand(emoji_scrolled, 1);

    let emoji_flow = gtk_flow_box_new();
    gtk_flow_box_set_homogeneous(emoji_flow, 1);
    gtk_flow_box_set_min_children_per_line(emoji_flow, 7);
    gtk_flow_box_set_max_children_per_line(emoji_flow, EMOJI_GRID_COLS as c_uint);
    gtk_flow_box_set_column_spacing(emoji_flow, 4);
    gtk_flow_box_set_row_spacing(emoji_flow, 4);
    gtk_flow_box_set_selection_mode(emoji_flow, GTK_SELECTION_SINGLE);
    gtk_flow_box_set_activate_on_single_click(emoji_flow, 1);
    gtk_widget_add_css_class(emoji_flow, c"clip-emoji-grid".as_ptr());
    set_margins(emoji_flow, 2, 8, 12, 12);

    gtk_scrolled_window_set_child(emoji_scrolled, emoji_flow);
    gtk_stack_add_named(emoji_stack, emoji_scrolled, c"grid".as_ptr());

    let emoji_empty = adw_status_page_new();
    adw_status_page_set_icon_name(emoji_empty, c"edit-find-symbolic".as_ptr());
    adw_status_page_set_title(emoji_empty, c"No Emojis Found".as_ptr());
    adw_status_page_set_description(
        emoji_empty,
        c"Try searching by name or keyword like smile, heart, or fire.".as_ptr(),
    );
    gtk_stack_add_named(emoji_stack, emoji_empty, c"empty".as_ptr());

    gtk_box_append(emoji_page, emoji_stack);
    gtk_stack_add_named(main_stack, emoji_page, c"emojis".as_ptr());

    gtk_box_append(root_box, main_stack);

    // Keyboard hint footer.
    let footer = gtk_label_new(c"".as_ptr());
    gtk_widget_add_css_class(footer, c"clip-footer".as_ptr());
    set_margins(footer, 4, 8, 12, 12);
    gtk_box_append(root_box, footer);

    adw_application_window_set_content(window, root_box);

    {
        let mut s = state_rc.borrow_mut();
        s.window = window;
        s.search_entry = search_entry;
        s.list_box = list_box;
        s.scrolled = scrolled;
        s.stack = stack;
        s.status_page = status_page;
        s.count_label = count_lbl;
        s.clear_btn = clear_btn;
        s.settings_revealer = settings_revealer;
        s.settings_btn = settings_btn;
        s.tab_clipboard_btn = tab_clipboard_btn;
        s.tab_emoji_btn = tab_emoji_btn;
        s.main_stack = main_stack;
        s.emoji_stack = emoji_stack;
        s.emoji_scrolled = emoji_scrolled;
        s.emoji_flow = emoji_flow;
        s.emoji_empty = emoji_empty;
        s.emoji_cat_btns = emoji_cat_btns;
        s.emoji_info_label = emoji_info_label;
        s.footer_label = footer;
    }

    connect_boxed(
        tab_clipboard_btn,
        c"toggled",
        on_tab_clipboard_toggled as GCallback,
        Rc::downgrade(state_rc),
    );
    connect_boxed(
        tab_emoji_btn,
        c"toggled",
        on_tab_emojis_toggled as GCallback,
        Rc::downgrade(state_rc),
    );
    connect_boxed(
        emoji_flow,
        c"child-activated",
        on_emoji_child_activated as GCallback,
        Rc::downgrade(state_rc),
    );

    connect_boxed(
        search_entry,
        c"changed",
        on_search_changed as GCallback,
        Rc::downgrade(state_rc),
    );
    connect_boxed(
        list_box,
        c"row-activated",
        on_row_activated as GCallback,
        Rc::downgrade(state_rc),
    );
    connect_boxed(
        clear_btn,
        c"clicked",
        on_clear_clicked as GCallback,
        Rc::downgrade(state_rc),
    );
    connect_boxed(
        settings_btn,
        c"clicked",
        on_settings_toggled as GCallback,
        Rc::downgrade(state_rc),
    );
    connect_boxed(
        window,
        c"notify::is-active",
        on_is_active_changed as GCallback,
        Rc::downgrade(state_rc),
    );
    connect_boxed(
        window,
        c"close-request",
        on_window_close_request as GCallback,
        Rc::downgrade(state_rc),
    );

    let key_ctrl = gtk_event_controller_key_new();
    gtk_event_controller_set_propagation_phase(key_ctrl, GTK_PHASE_CAPTURE);
    connect_boxed(
        key_ctrl,
        c"key-pressed",
        on_key_pressed as GCallback,
        Rc::downgrade(state_rc),
    );
    gtk_widget_add_controller(window, key_ctrl);

    apply_tab_chrome(
        state_rc,
        initial_tab,
        clear_btn,
        search_entry,
        main_stack,
        footer,
        tab_clipboard_btn,
        tab_emoji_btn,
    );
    gtk_window_present(window);
    gtk_widget_grab_focus(search_entry);
}

unsafe extern "C" fn on_activate(app: *mut c_void, user_data: *mut c_void) {
    let state_rc = &*(user_data as *const Rc<RefCell<UiState>>);
    let existing_win = state_rc.borrow().window;
    if !existing_win.is_null() {
        state_rc.borrow_mut().closing = true;
        gtk_window_close(existing_win);
        return;
    }
    open_window(app, state_rc, ActiveTab::Clipboard);
}

unsafe extern "C" fn on_command_line(
    app: *mut c_void,
    cmdline: *mut c_void,
    user_data: *mut c_void,
) -> c_int {
    let state_rc = &*(user_data as *const Rc<RefCell<UiState>>);
    let mut argc: c_int = 0;
    let argv = g_application_command_line_get_arguments(cmdline, &mut argc);
    let mut target_tab = ActiveTab::Clipboard;
    if !argv.is_null() {
        for i in 1..(argc.max(0) as isize) {
            let arg_ptr = *argv.offset(i);
            if !arg_ptr.is_null() {
                let arg = ptr_to_string(arg_ptr);
                if matches!(arg.as_str(), "--emojis" | "-e" | "emojis") {
                    target_tab = ActiveTab::Emojis;
                    break;
                }
            }
        }
        g_strfreev(argv);
    }

    let (existing_win, current_tab) = {
        let s = state_rc.borrow();
        (s.window, s.active_tab)
    };
    if !existing_win.is_null() {
        if current_tab == target_tab {
            state_rc.borrow_mut().closing = true;
            gtk_window_close(existing_win);
        } else {
            switch_tab(state_rc, target_tab);
            gtk_window_present(existing_win);
        }
    } else {
        open_window(app, state_rc, target_tab);
    }
    0
}

fn main() {
    let state = Rc::new(RefCell::new(UiState {
        socket_path: default_socket_path(),
        window: ptr::null_mut(),
        search_entry: ptr::null_mut(),
        list_box: ptr::null_mut(),
        scrolled: ptr::null_mut(),
        stack: ptr::null_mut(),
        status_page: ptr::null_mut(),
        count_label: ptr::null_mut(),
        clear_btn: ptr::null_mut(),
        settings_revealer: ptr::null_mut(),
        settings_btn: ptr::null_mut(),
        tab_clipboard_btn: ptr::null_mut(),
        tab_emoji_btn: ptr::null_mut(),
        main_stack: ptr::null_mut(),
        emoji_stack: ptr::null_mut(),
        emoji_scrolled: ptr::null_mut(),
        emoji_flow: ptr::null_mut(),
        emoji_empty: ptr::null_mut(),
        emoji_cat_btns: Vec::new(),
        emoji_info_label: ptr::null_mut(),
        footer_label: ptr::null_mut(),
        active_tab: ActiveTab::Clipboard,
        active_emoji_cat: EmojiCategory::Recent,
        recent_emojis: Vec::new(),
        visible_emojis: Vec::new(),
        selected_emoji_idx: 0,
        emoji_loaded: false,
        entries: Vec::new(),
        settings: Settings::load(),
        has_been_active: false,
        closing: false,
        pending_action: None,
    }));
    let state_after = Rc::clone(&state);

    let c_args: Vec<std::ffi::CString> = std::env::args().map(|a| cstr(&a)).collect();
    let mut argv_ptrs: Vec<*mut std::ffi::c_char> = c_args
        .iter()
        .map(|cs| cs.as_ptr() as *mut std::ffi::c_char)
        .collect();
    let argc = argv_ptrs.len() as c_int;
    argv_ptrs.push(ptr::null_mut());

    unsafe {
        let app = adw_application_new(
            c"org.clipboardlx.UI".as_ptr(),
            G_APPLICATION_HANDLES_COMMAND_LINE,
        );
        connect_boxed(
            app,
            c"command-line",
            on_command_line as GCallback,
            Rc::clone(&state),
        );
        connect_boxed(app, c"activate", on_activate as GCallback, state);
        let status = g_application_run(app, argc, argv_ptrs.as_mut_ptr());
        drop(c_args);
        g_object_unref(app);

        let (socket_path, action) = {
            let mut s = state_after.borrow_mut();
            (s.socket_path.clone(), s.pending_action.take())
        };
        match action {
            Some(PendingAction::SelectEntry { id, auto_paste }) => {
                if let Err(e) = send_request(&socket_path, &Request::Select { id, auto_paste }) {
                    eprintln!("clip-ui: select failed: {}", e);
                }
            }
            Some(PendingAction::PasteEmoji { emoji, auto_paste }) => {
                if let Err(e) =
                    send_request(&socket_path, &Request::PasteText { text: emoji, auto_paste })
                {
                    eprintln!("clip-ui: emoji paste failed: {}", e);
                }
            }
            None => {}
        }

        std::process::exit(status);
    }
}

#[cfg(test)]
mod tests {
    use super::preview_text;

    #[test]
    fn preview_collapses_whitespace_and_truncates_utf8() {
        assert_eq!(preview_text("  hello \n\t  world  "), "hello world");
        let long = "é".repeat(300);
        let p = preview_text(&long);
        assert!(p.ends_with('…'));
        assert_eq!(p.chars().count(), 241);
    }
}
