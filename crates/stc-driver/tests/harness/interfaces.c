/* M5 phase 4b.4 harness: interface values, upcasts, and dispatch. */

typedef int (*int_fn)(void);

extern int stc_host_load_module(const char* path, int mode);
extern int stc_host_cycle(void);
extern int stc_host_shutdown(void);
extern void* stc_host_symbol(const char* name);

int main(int argc, char** argv) {
  if (argc < 2) return 30;
  if (stc_host_load_module(argv[1], 0) != 0) return 31;

  int_fn dispatch = (int_fn)stc_host_symbol("RunDispatch");
  int_fn nul = (int_fn)stc_host_symbol("RunNull");
  int_fn arrays = (int_fn)stc_host_symbol("RunArrays");
  int_fn fields = (int_fn)stc_host_symbol("RunFields");
  int_fn index = (int_fn)stc_host_symbol("RunIndex");
  int_fn fb = (int_fn)stc_host_symbol("RunFb");
  int_fn check_main = (int_fn)stc_host_symbol("CheckMain");
  if (!dispatch) return 40;
  if (!nul) return 41;
  if (!arrays) return 42;
  if (!fields) return 43;
  if (!index) return 44;
  if (!fb) return 45;
  if (!check_main) return 46;

  if (dispatch() != 124) return 1;
  if (nul() != 1101) return 2;
  if (arrays() != 131) return 3;
  if (fields() != 258) return 4;
  if (index() != 60) return 5;
  if (fb() != 55) return 6;

  if (check_main() != 0) return 10;
  if (stc_host_cycle() != 0) return 20;
  if (check_main() != 300) return 21;
  if (stc_host_shutdown() != 0) return 22;
  return 0;
}
