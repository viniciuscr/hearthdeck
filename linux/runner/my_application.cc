#include "my_application.h"

#include <flutter_linux/flutter_linux.h>

#include "flutter/generated_plugin_registrant.h"

struct _MyApplication {
  GtkApplication parent_instance;
  char** dart_entrypoint_arguments;
  FlMethodChannel* session_channel;
};

static constexpr char kSessionChannel[] =
    "io.github.viniciuscr.hearthdeck/session";

static void session_method_call_cb(FlMethodChannel* channel,
                                   FlMethodCall* method_call,
                                   gpointer user_data) {
  (void)channel;
  MyApplication* self = MY_APPLICATION(user_data);
  const gchar* method = fl_method_call_get_name(method_call);
  if (g_strcmp0(method, "exitToDesktop") == 0) {
    fl_method_call_respond_success(method_call, nullptr, nullptr);
    g_application_quit(G_APPLICATION(self));
    return;
  }
  fl_method_call_respond_not_implemented(method_call, nullptr);
}

G_DEFINE_TYPE(MyApplication, my_application, GTK_TYPE_APPLICATION)

// Called when first Flutter frame received.
static void first_frame_cb(MyApplication* self, FlView* view) {
  gtk_widget_show(gtk_widget_get_toplevel(GTK_WIDGET(view)));
}

// Whether `XDG_CURRENT_DESKTOP` (colon-separated per the XDG Desktop Entry
// spec) lists `name`, case-insensitively. Mirrors hearthdeck-bridge's
// current_desktops()/is_kiosk_session() (services/hearthdeck-bridge/src/
// platform/linux.rs), which parses the same variable the same way.
static gboolean session_has_desktop_name(const char* name) {
  const char* value = g_getenv("XDG_CURRENT_DESKTOP");
  if (value == nullptr) {
    return FALSE;
  }
  g_auto(GStrv) parts = g_strsplit(value, ":", -1);
  for (gchar** part = parts; *part != nullptr; part++) {
    if (g_ascii_strcasecmp(*part, name) == 0) {
      return TRUE;
    }
  }
  return FALSE;
}

// Implements GApplication::activate.
static void my_application_activate(GApplication* application) {
  MyApplication* self = MY_APPLICATION(application);
  GtkWindow* window =
      GTK_WINDOW(gtk_application_window_new(GTK_APPLICATION(application)));
  gtk_window_set_title(window, "hearthdeck");
  gtk_window_set_decorated(window, FALSE);
  // On Wayland, gtk_window_set_decorated(FALSE) alone does not stop the
  // compositor from drawing its own server-side title bar/min/max/close
  // buttons: GTK only tells the compositor to skip that (announce_csd
  // instead of announce_ssd, in GTK3's gdkwindow-wayland.c) when the window
  // has a client-side titlebar widget installed via
  // gtk_window_set_titlebar(), regardless of the decorated property. This
  // matters once the window is undecorated but not fullscreen (as in the
  // COSMIC (Test) session below, which maximizes instead) - a fullscreen
  // window happens to never get compositor decorations either way, which is
  // why this went unnoticed in the Kiosk (Gamescope) session. An empty
  // titlebar widget satisfies GTK's requirement with nothing rendered.
  gtk_window_set_titlebar(window, gtk_grid_new());
  // Xvfb plus a minimal, non-compositing window manager (this repo's
  // docker/ visual test sandbox) can leave gtk_window_fullscreen()'s async
  // round-trip to the WM never completing, so the GL area never learns its
  // target size and the engine waits forever for a first frame that never
  // renders. Real hardware doesn't hit this -- Gamescope owns the whole DRM
  // output and sizes the window synchronously -- so only bypass fullscreen
  // for that specific sandbox, never for a real session.
  if (g_getenv("HEARTHDECK_DOCKER_SANDBOX") != nullptr) {
    gtk_window_set_default_size(window, 1920, 1080);
  } else if (session_has_desktop_name("COSMIC")) {
    // The COSMIC (Test) session (packaging/arch/cosmic-test-session) runs
    // cosmic-panel as a real top bar. A fullscreen window overlaps it and
    // trips the panel's autohide-on-overlap, hiding the bar while
    // Hearthdeck itself is in view -- exactly backwards. Maximizing
    // instead respects the panel's reserved screen edge; cosmic-comp sizes
    // a maximized toplevel to the work area outside that reservation.
    // Launched games/apps are unaffected: they still go fullscreen on
    // their own and trigger the autohide as intended.
    gtk_window_maximize(window);
  } else {
    gtk_window_fullscreen(window);
  }

  g_autoptr(FlDartProject) project = fl_dart_project_new();
  fl_dart_project_set_dart_entrypoint_arguments(
      project, self->dart_entrypoint_arguments);

  FlView* view = fl_view_new(project);
  g_autoptr(FlStandardMethodCodec) codec = fl_standard_method_codec_new();
  self->session_channel = fl_method_channel_new(
      fl_engine_get_binary_messenger(fl_view_get_engine(view)), kSessionChannel,
      FL_METHOD_CODEC(codec));
  fl_method_channel_set_method_call_handler(self->session_channel,
                                             session_method_call_cb, self,
                                             nullptr);
  GdkRGBA background_color;
  // Background defaults to black, override it here if necessary, e.g. #00000000
  // for transparent.
  gdk_rgba_parse(&background_color, "#000000");
  fl_view_set_background_color(view, &background_color);
  gtk_widget_show(GTK_WIDGET(view));
  gtk_container_add(GTK_CONTAINER(window), GTK_WIDGET(view));

  // Show the window when Flutter renders.
  // Requires the view to be realized so we can start rendering.
  g_signal_connect_swapped(view, "first-frame", G_CALLBACK(first_frame_cb),
                           self);
  gtk_widget_realize(GTK_WIDGET(view));

  fl_register_plugins(FL_PLUGIN_REGISTRY(view));

  gtk_widget_grab_focus(GTK_WIDGET(view));
}

// Implements GApplication::local_command_line.
static gboolean my_application_local_command_line(GApplication* application,
                                                  gchar*** arguments,
                                                  int* exit_status) {
  MyApplication* self = MY_APPLICATION(application);
  // Strip out the first argument as it is the binary name.
  self->dart_entrypoint_arguments = g_strdupv(*arguments + 1);

  g_autoptr(GError) error = nullptr;
  if (!g_application_register(application, nullptr, &error)) {
    g_warning("Failed to register: %s", error->message);
    *exit_status = 1;
    return TRUE;
  }

  g_application_activate(application);
  *exit_status = 0;

  return TRUE;
}

// In the Hearthdeck Kiosk session, Gamescope execs this process directly and
// assigns DISPLAY/WAYLAND_DISPLAY only to this process's own environment -
// never to the systemd --user manager's shared activation environment, and
// never to hearthdeck-session's own script (it hands off control via `exec
// gamescope` before Gamescope even creates these), so this is the only place
// that ever legitimately has the correct values. Importing them here, the
// same way hearthdeck-session already imports XDG_CURRENT_DESKTOP and
// friends, is what lets hearthdeck-bridge forward a real, working display
// connection to every app/game it launches (see launch_with_systemd in
// hearthdeck-bridge's linux.rs) instead of guessing or going without.
// Harmless (and redundant, since a normal desktop session already has these
// correctly) outside the Kiosk session too, so this always runs
// unconditionally rather than trying to detect which case this is.
static void import_display_environment() {
  g_autoptr(GError) error = nullptr;
  if (!g_spawn_command_line_async(
          "systemctl --user import-environment DISPLAY WAYLAND_DISPLAY",
          &error)) {
    g_warning("Failed to import DISPLAY/WAYLAND_DISPLAY into systemd --user: %s",
              error->message);
  }

  // hearthdeck-bridge.service is socket-activated and may not have started
  // yet at this point - in which case its first activation naturally picks
  // up the environment just imported above, no restart needed. This is
  // cheap insurance against it having started earlier with a stale
  // pre-import environment, mirroring hearthdeck-session's own
  // try-restart-after-import step for the same reason.
  g_autoptr(GError) restart_error = nullptr;
  if (!g_spawn_command_line_async(
          "systemctl --user try-restart hearthdeck-bridge.service",
          &restart_error)) {
    g_warning("Failed to restart hearthdeck-bridge.service after importing "
              "display environment: %s",
              restart_error->message);
  }
}

// Implements GApplication::startup.
static void my_application_startup(GApplication* application) {
  // MyApplication* self = MY_APPLICATION(object);

  import_display_environment();

  G_APPLICATION_CLASS(my_application_parent_class)->startup(application);
}

// Implements GApplication::shutdown.
static void my_application_shutdown(GApplication* application) {
  // MyApplication* self = MY_APPLICATION(object);

  // Perform any actions required at application shutdown.

  G_APPLICATION_CLASS(my_application_parent_class)->shutdown(application);
}

// Implements GObject::dispose.
static void my_application_dispose(GObject* object) {
  MyApplication* self = MY_APPLICATION(object);
  g_clear_pointer(&self->dart_entrypoint_arguments, g_strfreev);
  g_clear_object(&self->session_channel);
  G_OBJECT_CLASS(my_application_parent_class)->dispose(object);
}

static void my_application_class_init(MyApplicationClass* klass) {
  G_APPLICATION_CLASS(klass)->activate = my_application_activate;
  G_APPLICATION_CLASS(klass)->local_command_line =
      my_application_local_command_line;
  G_APPLICATION_CLASS(klass)->startup = my_application_startup;
  G_APPLICATION_CLASS(klass)->shutdown = my_application_shutdown;
  G_OBJECT_CLASS(klass)->dispose = my_application_dispose;
}

static void my_application_init(MyApplication* self) {}

MyApplication* my_application_new() {
  // Set the program name to the application ID, which helps various systems
  // like GTK and desktop environments map this running application to its
  // corresponding .desktop file. This ensures better integration by allowing
  // the application to be recognized beyond its binary name.
  g_set_prgname(APPLICATION_ID);

  return MY_APPLICATION(g_object_new(my_application_get_type(),
                                     "application-id", APPLICATION_ID, "flags",
                                     G_APPLICATION_NON_UNIQUE, nullptr));
}
