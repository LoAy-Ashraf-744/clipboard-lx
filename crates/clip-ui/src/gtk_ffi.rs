#![allow(dead_code)]

use std::ffi::{c_char, c_int, c_uint, c_ulong, c_void, CStr, CString};
use std::ptr;

pub const GTK_ORIENTATION_HORIZONTAL: c_int = 0;
pub const GTK_ORIENTATION_VERTICAL: c_int = 1;

pub const GTK_ALIGN_FILL: c_int = 0;
pub const GTK_ALIGN_START: c_int = 1;
pub const GTK_ALIGN_END: c_int = 2;
pub const GTK_ALIGN_CENTER: c_int = 3;

pub const GTK_SELECTION_SINGLE: c_int = 1;
pub const GTK_PHASE_CAPTURE: c_int = 1;
pub const GTK_POLICY_AUTOMATIC: c_int = 1;
pub const GTK_POLICY_NEVER: c_int = 2;
pub const GTK_CONTENT_FIT_CONTAIN: c_int = 1;
pub const GTK_CONTENT_FIT_COVER: c_int = 2;
pub const GTK_OVERFLOW_HIDDEN: c_int = 1;

pub const PANGO_ELLIPSIZE_END: c_int = 3;
pub const GTK_STYLE_PROVIDER_PRIORITY_APPLICATION: c_uint = 600;
pub const G_APPLICATION_DEFAULT_FLAGS: c_int = 0;
pub const G_APPLICATION_HANDLES_COMMAND_LINE: c_int = 8;

pub const GDK_CONTROL_MASK: c_uint = 4;
pub const GDK_ALT_MASK: c_uint = 8;
pub const GDK_KEY_ESCAPE: c_uint = 65307;
pub const GDK_KEY_RETURN: c_uint = 65293;
pub const GDK_KEY_KP_ENTER: c_uint = 65421;
pub const GDK_KEY_LEFT: c_uint = 65361;
pub const GDK_KEY_UP: c_uint = 65362;
pub const GDK_KEY_RIGHT: c_uint = 65363;
pub const GDK_KEY_DOWN: c_uint = 65364;
pub const GDK_KEY_TAB: c_uint = 65289;
pub const GDK_KEY_ISO_LEFT_TAB: c_uint = 65056;
pub const GDK_KEY_1: c_uint = 49;
pub const GDK_KEY_2: c_uint = 50;
pub const GDK_KEY_DELETE: c_uint = 65535;
pub const GDK_KEY_P_LOWER: c_uint = 112;
pub const GDK_KEY_P_UPPER: c_uint = 80;

pub type GCallback = *const c_void;
pub type GClosureNotify = Option<unsafe extern "C" fn(*mut c_void, *mut c_void)>;

#[repr(C)]
#[derive(Default, Copy, Clone)]
pub struct GrapheneRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

extern "C" {
    // GLib / GObject / Gio
    pub fn g_object_unref(object: *mut c_void);
    pub fn g_bytes_new(data: *const c_void, size: usize) -> *mut c_void;
    pub fn g_bytes_unref(bytes: *mut c_void);
    pub fn g_error_free(error: *mut c_void);

    pub fn g_signal_connect_data(
        instance: *mut c_void,
        detailed_signal: *const c_char,
        c_handler: GCallback,
        data: *mut c_void,
        destroy_data: GClosureNotify,
        connect_flags: c_int,
    ) -> c_ulong;

    pub fn g_application_run(
        application: *mut c_void,
        argc: c_int,
        argv: *mut *mut c_char,
    ) -> c_int;
    pub fn g_application_command_line_get_arguments(
        cmdline: *mut c_void,
        argc: *mut c_int,
    ) -> *mut *mut c_char;
    pub fn g_strfreev(str_array: *mut *mut c_char);

    // Libadwaita
    pub fn adw_application_new(application_id: *const c_char, flags: c_int) -> *mut c_void;
    pub fn adw_application_window_new(app: *mut c_void) -> *mut c_void;
    pub fn adw_application_window_set_content(window: *mut c_void, content: *mut c_void);

    pub fn adw_header_bar_new() -> *mut c_void;
    pub fn adw_header_bar_pack_start(bar: *mut c_void, child: *mut c_void);
    pub fn adw_header_bar_pack_end(bar: *mut c_void, child: *mut c_void);
    pub fn adw_header_bar_set_title_widget(bar: *mut c_void, title_widget: *mut c_void);

    pub fn adw_status_page_new() -> *mut c_void;
    pub fn adw_status_page_set_icon_name(page: *mut c_void, icon_name: *const c_char);
    pub fn adw_status_page_set_title(page: *mut c_void, title: *const c_char);
    pub fn adw_status_page_set_description(page: *mut c_void, description: *const c_char);

    // GDK
    pub fn gdk_display_get_default() -> *mut c_void;
    pub fn gdk_texture_new_from_bytes(
        bytes: *mut c_void,
        error: *mut *mut c_void,
    ) -> *mut c_void;

    // GTK4 Window / Widget
    pub fn gtk_window_set_title(window: *mut c_void, title: *const c_char);
    pub fn gtk_window_set_default_size(window: *mut c_void, width: c_int, height: c_int);
    pub fn gtk_window_present(window: *mut c_void);
    pub fn gtk_window_close(window: *mut c_void);
    pub fn gtk_window_is_active(window: *mut c_void) -> c_int;

    pub fn gtk_widget_grab_focus(widget: *mut c_void) -> c_int;
    pub fn gtk_widget_set_visible(widget: *mut c_void, visible: c_int);
    pub fn gtk_widget_add_css_class(widget: *mut c_void, css_class: *const c_char);
    pub fn gtk_widget_remove_css_class(widget: *mut c_void, css_class: *const c_char);
    pub fn gtk_widget_set_margin_top(widget: *mut c_void, margin: c_int);
    pub fn gtk_widget_set_margin_bottom(widget: *mut c_void, margin: c_int);
    pub fn gtk_widget_set_margin_start(widget: *mut c_void, margin: c_int);
    pub fn gtk_widget_set_margin_end(widget: *mut c_void, margin: c_int);
    pub fn gtk_widget_set_hexpand(widget: *mut c_void, expand: c_int);
    pub fn gtk_widget_set_vexpand(widget: *mut c_void, expand: c_int);
    pub fn gtk_widget_set_halign(widget: *mut c_void, align: c_int);
    pub fn gtk_widget_set_valign(widget: *mut c_void, align: c_int);
    pub fn gtk_widget_set_size_request(widget: *mut c_void, width: c_int, height: c_int);
    pub fn gtk_widget_set_tooltip_text(widget: *mut c_void, text: *const c_char);
    pub fn gtk_widget_set_focusable(widget: *mut c_void, focusable: c_int);
    pub fn gtk_widget_set_overflow(widget: *mut c_void, overflow: c_int);
    pub fn gtk_widget_add_controller(widget: *mut c_void, controller: *mut c_void);
    pub fn gtk_widget_compute_bounds(
        widget: *mut c_void,
        target: *mut c_void,
        out_bounds: *mut GrapheneRect,
    ) -> c_int;

    // CSS
    pub fn gtk_css_provider_new() -> *mut c_void;
    pub fn gtk_css_provider_load_from_string(provider: *mut c_void, string: *const c_char);
    pub fn gtk_style_context_add_provider_for_display(
        display: *mut c_void,
        provider: *mut c_void,
        priority: c_uint,
    );

    // Containers & Stack
    pub fn gtk_box_new(orientation: c_int, spacing: c_int) -> *mut c_void;
    pub fn gtk_box_append(box_: *mut c_void, child: *mut c_void);

    pub fn gtk_scrolled_window_new() -> *mut c_void;
    pub fn gtk_scrolled_window_set_child(sw: *mut c_void, child: *mut c_void);
    pub fn gtk_scrolled_window_set_policy(
        sw: *mut c_void,
        hscrollbar_policy: c_int,
        vscrollbar_policy: c_int,
    );
    pub fn gtk_scrolled_window_get_vadjustment(sw: *mut c_void) -> *mut c_void;
    pub fn gtk_adjustment_get_value(adj: *mut c_void) -> f64;
    pub fn gtk_adjustment_set_value(adj: *mut c_void, value: f64);
    pub fn gtk_adjustment_get_page_size(adj: *mut c_void) -> f64;

    pub fn gtk_stack_new() -> *mut c_void;
    pub fn gtk_stack_add_named(
        stack: *mut c_void,
        child: *mut c_void,
        name: *const c_char,
    ) -> *mut c_void;
    pub fn gtk_stack_set_visible_child_name(stack: *mut c_void, name: *const c_char);

    // Search & Controls
    pub fn gtk_search_entry_new() -> *mut c_void;
    pub fn gtk_search_entry_set_placeholder_text(entry: *mut c_void, text: *const c_char);
    pub fn gtk_editable_get_text(editable: *mut c_void) -> *const c_char;
    pub fn gtk_editable_set_text(editable: *mut c_void, text: *const c_char);

    pub fn gtk_label_new(str_: *const c_char) -> *mut c_void;
    pub fn gtk_label_set_text(label: *mut c_void, str_: *const c_char);
    pub fn gtk_label_set_markup(label: *mut c_void, str_: *const c_char);
    pub fn gtk_label_set_xalign(label: *mut c_void, xalign: f32);
    pub fn gtk_label_set_wrap(label: *mut c_void, wrap: c_int);
    pub fn gtk_label_set_lines(label: *mut c_void, lines: c_int);
    pub fn gtk_label_set_ellipsize(label: *mut c_void, mode: c_int);

    pub fn gtk_image_new_from_icon_name(icon_name: *const c_char) -> *mut c_void;
    pub fn gtk_image_set_pixel_size(image: *mut c_void, pixel_size: c_int);

    pub fn gtk_picture_new_for_paintable(paintable: *mut c_void) -> *mut c_void;
    pub fn gtk_picture_set_content_fit(picture: *mut c_void, content_fit: c_int);

    pub fn gtk_button_new_from_icon_name(icon_name: *const c_char) -> *mut c_void;
    pub fn gtk_button_new_with_label(label: *const c_char) -> *mut c_void;
    pub fn gtk_button_set_child(button: *mut c_void, child: *mut c_void);

    pub fn gtk_toggle_button_new() -> *mut c_void;
    pub fn gtk_toggle_button_set_active(button: *mut c_void, is_active: c_int);
    pub fn gtk_toggle_button_get_active(button: *mut c_void) -> c_int;
    pub fn gtk_toggle_button_set_group(button: *mut c_void, group: *mut c_void);

    // FlowBox
    pub fn gtk_flow_box_new() -> *mut c_void;
    pub fn gtk_flow_box_set_homogeneous(flow_box: *mut c_void, homogeneous: c_int);
    pub fn gtk_flow_box_set_min_children_per_line(flow_box: *mut c_void, n_children: c_uint);
    pub fn gtk_flow_box_set_max_children_per_line(flow_box: *mut c_void, n_children: c_uint);
    pub fn gtk_flow_box_set_column_spacing(flow_box: *mut c_void, spacing: c_uint);
    pub fn gtk_flow_box_set_row_spacing(flow_box: *mut c_void, spacing: c_uint);
    pub fn gtk_flow_box_set_selection_mode(flow_box: *mut c_void, mode: c_int);
    pub fn gtk_flow_box_set_activate_on_single_click(flow_box: *mut c_void, single: c_int);
    pub fn gtk_flow_box_append(flow_box: *mut c_void, child: *mut c_void);
    pub fn gtk_flow_box_remove_all(flow_box: *mut c_void);
    pub fn gtk_flow_box_get_child_at_index(flow_box: *mut c_void, idx: c_int) -> *mut c_void;
    pub fn gtk_flow_box_select_child(flow_box: *mut c_void, child: *mut c_void);
    pub fn gtk_flow_box_child_new() -> *mut c_void;
    pub fn gtk_flow_box_child_set_child(child: *mut c_void, widget: *mut c_void);
    pub fn gtk_flow_box_child_get_index(child: *mut c_void) -> c_int;

    pub fn gtk_revealer_new() -> *mut c_void;
    pub fn gtk_revealer_set_child(revealer: *mut c_void, child: *mut c_void);
    pub fn gtk_revealer_set_reveal_child(revealer: *mut c_void, reveal_child: c_int);
    pub fn gtk_revealer_get_reveal_child(revealer: *mut c_void) -> c_int;

    pub fn gtk_switch_new() -> *mut c_void;
    pub fn gtk_switch_set_active(sw: *mut c_void, is_active: c_int);
    pub fn gtk_switch_get_active(sw: *mut c_void) -> c_int;

    // ListBox
    pub fn gtk_list_box_new() -> *mut c_void;
    pub fn gtk_list_box_set_selection_mode(box_: *mut c_void, mode: c_int);
    pub fn gtk_list_box_set_activate_on_single_click(box_: *mut c_void, single: c_int);
    pub fn gtk_list_box_append(box_: *mut c_void, child: *mut c_void);
    pub fn gtk_list_box_remove_all(box_: *mut c_void);
    pub fn gtk_list_box_select_row(box_: *mut c_void, row: *mut c_void);
    pub fn gtk_list_box_get_selected_row(box_: *mut c_void) -> *mut c_void;
    pub fn gtk_list_box_get_row_at_index(box_: *mut c_void, index: c_int) -> *mut c_void;
    pub fn gtk_list_box_row_new() -> *mut c_void;
    pub fn gtk_list_box_row_set_child(row: *mut c_void, child: *mut c_void);
    pub fn gtk_list_box_row_get_index(row: *mut c_void) -> c_int;

    // EventControllerKey
    pub fn gtk_event_controller_key_new() -> *mut c_void;
    pub fn gtk_event_controller_set_propagation_phase(controller: *mut c_void, phase: c_int);
}

pub fn cstr(s: &str) -> CString {
    CString::new(s).unwrap_or_else(|_| CString::new(s.replace('\0', "")).unwrap())
}

pub unsafe fn ptr_to_string(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

pub unsafe extern "C" fn drop_boxed<T>(data: *mut c_void, _closure: *mut c_void) {
    if !data.is_null() {
        drop(Box::from_raw(data as *mut T));
    }
}

pub unsafe fn connect_boxed<T>(
    instance: *mut c_void,
    signal: &CStr,
    handler: GCallback,
    data: T,
) {
    let raw = Box::into_raw(Box::new(data)) as *mut c_void;
    g_signal_connect_data(
        instance,
        signal.as_ptr(),
        handler,
        raw,
        Some(drop_boxed::<T>),
        0,
    );
}

pub unsafe fn set_margins(w: *mut c_void, top: c_int, bottom: c_int, start: c_int, end: c_int) {
    gtk_widget_set_margin_top(w, top);
    gtk_widget_set_margin_bottom(w, bottom);
    gtk_widget_set_margin_start(w, start);
    gtk_widget_set_margin_end(w, end);
}

pub unsafe fn make_texture_from_png(png: &[u8]) -> *mut c_void {
    if png.is_empty() {
        return ptr::null_mut();
    }
    let bytes = g_bytes_new(png.as_ptr() as *const c_void, png.len());
    if bytes.is_null() {
        return ptr::null_mut();
    }
    let mut err: *mut c_void = ptr::null_mut();
    let texture = gdk_texture_new_from_bytes(bytes, &mut err);
    g_bytes_unref(bytes);
    if !err.is_null() {
        g_error_free(err);
    }
    texture
}
