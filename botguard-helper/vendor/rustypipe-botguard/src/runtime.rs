use deno_core::extension;
use deno_web::TimersPermission;

pub struct TPerm;

impl TimersPermission for TPerm {
    fn allow_hrtime(&mut self) -> bool {
        true
    }
}

extension!(
  runtime,
  deps = [deno_webidl, deno_console, deno_url, deno_web],
  esm_entry_point = "ext:runtime/99_main.js",
  esm = [
      dir "js",
      "98_global_scope_shared.js",
      "98_global_scope_window.js",
      "99_main.js"
  ],
  state = |state| {
    state.put(TPerm {});
  },
);
