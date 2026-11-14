mod gtk_ffi;

use std::cell::RefCell;
use std::ffi::{c_int, c_uint, c_void};
use std::path::PathBuf;
use std::ptr;
use std::rc::{Rc, Weak};

use clip_core::ipc::{send_request, Request, Response};
use clip_core::markdown::{is_markdown, markdown_to_pango_markup};
use clip_core::model::{format_byte_size, format_relative_time, now_millis, ClipboardEntry, EntryKind};
use clip_core::paths::default_socket_path;

use crate::gtk_ffi::*;

const MAX_DISPLAY_ITEMS: u32 = 100;

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
";

struct UiState {
    socket_path: PathBuf,
    window: *mut c_void,
    search_entry: *mut c_void,
    list_box: *mut c_void,
    scrolled: *mut c_void,
    stack: *mut c_void,
    status_page: *mut c_void,
    count_label: *mut c_void,
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
) -> *mut c_void {
    let row = gtk_list_box_row_new();
    let hbox = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 10);
    set_margins(hbox, 6, 6, 8, 6);

    let is_md = entry.kind == EntryKind::Text && is_markdown(&entry.text);

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
            for entry in &entries {
                let row = build_row_widget(state_rc, entry, now_ms);
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

unsafe fn select_index(state_rc: &Rc<RefCell<UiState>>, index: i32) {
    let (socket_path, id_opt, window) = {
        let s = state_rc.borrow();
        let id_opt = usize::try_from(index)
            .ok()
            .and_then(|i| s.entries.get(i))
            .map(|e| e.id);
        (s.socket_path.clone(), id_opt, s.window)
    };
    if let Some(id) = id_opt {
        let _ = send_request(&socket_path, &Request::Select { id });
        if !window.is_null() {
            gtk_window_close(window);
        }
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
        let vadj = gtk_scrolled_window_get_vadjustment(s.scrolled);
        if !vadj.is_null() {
            let page = gtk_adjustment_get_page_size(vadj);
            let cur_scroll = gtk_adjustment_get_value(vadj);
            let mut bounds = GrapheneRect::default();
            let (top, bottom) = if gtk_widget_compute_bounds(row, s.list_box, &mut bounds) != 0 {
                (bounds.y as f64, (bounds.y + bounds.height) as f64)
            } else {
                let t = (next as f64) * 66.0;
                (t, t + 66.0)
            };
            if top < cur_scroll {
                gtk_adjustment_set_value(vadj, top);
            } else if bottom > cur_scroll + page {
                gtk_adjustment_set_value(vadj, bottom - page);
            }
        }
    }
}

unsafe extern "C" fn on_search_changed(_entry: *mut c_void, user_data: *mut c_void) {
    let weak = &*(user_data as *const Weak<RefCell<UiState>>);
    if let Some(state_rc) = weak.upgrade() {
        refresh_list(&state_rc);
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

    match keyval {
        GDK_KEY_ESCAPE => {
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
            1
        }
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

unsafe extern "C" fn on_window_close_request(_win: *mut c_void, user_data: *mut c_void) -> c_int {
    let weak = &*(user_data as *const Weak<RefCell<UiState>>);
    if let Some(state_rc) = weak.upgrade() {
        state_rc.borrow_mut().window = ptr::null_mut();
    }
    0
}

unsafe extern "C" fn on_activate(app: *mut c_void, user_data: *mut c_void) {
    let state_rc = &*(user_data as *const Rc<RefCell<UiState>>);

    // Toggle off if already open (pressing Super+V twice hides it).
    let existing_win = state_rc.borrow().window;
    if !existing_win.is_null() {
        gtk_window_close(existing_win);
        return;
    }

    install_css();

    let window = adw_application_window_new(app);
    gtk_window_set_title(window, c"Clipboard".as_ptr());
    gtk_window_set_default_size(window, 480, 560);

    let root_box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);

    // Native GNOME/Ubuntu HeaderBar.
    let header = adw_header_bar_new();
    let title_lbl = gtk_label_new(c"Clipboard".as_ptr());
    gtk_widget_add_css_class(title_lbl, c"title".as_ptr());
    adw_header_bar_set_title_widget(header, title_lbl);

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

    gtk_box_append(root_box, header);

    // Search box directly below header.
    let search_entry = gtk_search_entry_new();
    gtk_search_entry_set_placeholder_text(search_entry, c"Search copied text or images…".as_ptr());
    gtk_widget_add_css_class(search_entry, c"clip-search".as_ptr());
    set_margins(search_entry, 8, 6, 12, 12);
    gtk_box_append(root_box, search_entry);

    // Main stack: List vs Empty/Offline StatusPage.
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

    gtk_box_append(root_box, stack);

    // Keyboard hint footer.
    let footer = gtk_label_new(
        c"↑↓ Navigate · Enter Paste · Ctrl+P Pin · Del Remove · Esc Close".as_ptr(),
    );
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
    }

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

    refresh_list(state_rc);
    gtk_window_present(window);
    gtk_widget_grab_focus(search_entry);
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
        entries: Vec::new(),
    }));

    unsafe {
        let app = adw_application_new(
            c"org.clipboardlx.UI".as_ptr(),
            G_APPLICATION_DEFAULT_FLAGS,
        );
        connect_boxed(app, c"activate", on_activate as GCallback, state);
        let status = g_application_run(app, 0, ptr::null_mut());
        g_object_unref(app);
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
