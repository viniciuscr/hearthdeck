#include "my_application.h"

#include <flutter_linux/flutter_linux.h>
#include <glib/gstdio.h>

#include "flutter/generated_plugin_registrant.h"

struct _MyApplication {
  GtkApplication parent_instance;
  char** dart_entrypoint_arguments;
  FlMethodChannel* session_channel;
};

static constexpr char kSessionChannel[] =
    "io.github.viniciuscr.hearthdeck/session";

static gchar* gamescope_wayland_display_path() {
  const gchar* runtime_directory = g_getenv("XDG_RUNTIME_DIR");
  if (runtime_directory == nullptr || *runtime_directory == '\0') {
    return nullptr;
  }
  return g_build_filename(runtime_directory, "hearthdeck",
                          "gamescope-wayland-display", nullptr);
}

// Records this process's own Wayland socket (Gamescope's, in the Kiosk
// session; whatever desktop compositor's, otherwise) to a runtime file. The
// bridge reads this file so a game/app it launches later, in its own
// transient systemd unit, can be pointed at the same compositor Hearthdeck
// itself is running in without needing its own copy of that environment
// variable. This is unrelated to, and does not require, the overlay
// (services/hearthdeck-overlay): it is plumbing for nested game launches.
static void publish_gamescope_wayland_display() {
  const gchar* wayland_display = g_getenv("WAYLAND_DISPLAY");
  if (wayland_display == nullptr || *wayland_display == '\0') {
    return;
  }

  g_autofree gchar* path = gamescope_wayland_display_path();
  if (path == nullptr) {
    return;
  }
  g_autoptr(GError) error = nullptr;
  g_autofree gchar* directory = g_path_get_dirname(path);
  if (g_mkdir_with_parents(directory, 0700) != 0 ||
      !g_file_set_contents(path, wayland_display, -1, &error)) {
    g_warning("Could not publish Gamescope Wayland socket: %s",
              error == nullptr ? "unknown error" : error->message);
  }
}

static void clear_gamescope_wayland_display() {
  g_autofree gchar* path = gamescope_wayland_display_path();
  if (path != nullptr) {
    g_remove(path);
  }
}

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
  publish_gamescope_wayland_display();
}

// Implements GApplication::activate.
static void my_application_activate(GApplication* application) {
  MyApplication* self = MY_APPLICATION(application);
  GtkWindow* window =
      GTK_WINDOW(gtk_application_window_new(GTK_APPLICATION(application)));
  gtk_window_set_title(window, "hearthdeck");
  gtk_window_set_decorated(window, FALSE);
  gtk_window_fullscreen(window);

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

// Implements GApplication::startup.
static void my_application_startup(GApplication* application) {
  // MyApplication* self = MY_APPLICATION(object);

  // Perform any actions required at application startup.

  G_APPLICATION_CLASS(my_application_parent_class)->startup(application);
}

// Implements GApplication::shutdown.
static void my_application_shutdown(GApplication* application) {
  clear_gamescope_wayland_display();

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
