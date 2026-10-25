/* M5 phase 4b.5 harness: assignment attempts, object references, and
   REF_TO-to-interface conversions. */

typedef int (*int_fn)(void);

extern int stc_host_load_module(const char* path, int mode);
extern int stc_host_cycle(void);
extern int stc_host_shutdown(void);
extern void* stc_host_symbol(const char* name);

int main(int argc, char** argv) {
  if (argc < 2) return 30;
  if (stc_host_load_module(argv[1], 0) != 0) return 31;

  int_fn attempts = (int_fn)stc_host_symbol("RunAttempts");
  int_fn refs = (int_fn)stc_host_symbol("RunObjectRefs");
  int_fn ref_iface = (int_fn)stc_host_symbol("RunRefInterface");
  int_fn ref_attempts = (int_fn)stc_host_symbol("RunRefAttempts");
  if (!attempts) return 40;
  if (!refs) return 41;
  if (!ref_iface) return 42;
  if (!ref_attempts) return 43;

  if (attempts() != 11111134) return 1;
  if (refs() != 1230) return 2;
  if (ref_iface() != 101118) return 3;
  if (ref_attempts() != 1114) return 4;

  if (stc_host_cycle() != 0) return 20;
  if (stc_host_shutdown() != 0) return 21;
  return 0;
}
