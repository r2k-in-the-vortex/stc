/* M5 phase 4b.3 harness: object records, virtual dispatch, and handle copy. */

typedef int (*int_fn)(void);

extern int stc_host_load_module(const char* path, int mode);
extern int stc_host_cycle(void);
extern int stc_host_shutdown(void);
extern void* stc_host_symbol(const char* name);

int main(int argc, char** argv) {
  if (argc < 2) return 30;
  if (stc_host_load_module(argv[1], 0) != 0) return 31;

  int_fn basic = (int_fn)stc_host_symbol("RunBasic");
  int_fn copy = (int_fn)stc_host_symbol("RunCopy");
  int_fn array = (int_fn)stc_host_symbol("RunArray");
  int_fn nested = (int_fn)stc_host_symbol("RunNested");
  int_fn global = (int_fn)stc_host_symbol("RunGlobal");
  int_fn rec = (int_fn)stc_host_symbol("RunRec");
  int_fn fb = (int_fn)stc_host_symbol("RunFb");
  int_fn param = (int_fn)stc_host_symbol("RunParam");
  int_fn nul = (int_fn)stc_host_symbol("RunNull");
  int_fn fbout = (int_fn)stc_host_symbol("RunFbOut");
  int_fn check_main = (int_fn)stc_host_symbol("CheckMain");
  int_fn strct = (int_fn)stc_host_symbol("RunStruct");
  int_fn gstruct = (int_fn)stc_host_symbol("RunGlobalStruct");
  int_fn abstrct = (int_fn)stc_host_symbol("RunAbstract");
  int_fn fbsuper = (int_fn)stc_host_symbol("RunFbSuper");
  int_fn pool = (int_fn)stc_host_symbol("RunPool");
  if (!basic) return 40;
  if (!copy) return 41;
  if (!array) return 42;
  if (!nested) return 43;
  if (!global) return 44;
  if (!rec) return 45;
  if (!fb) return 46;
  if (!param) return 47;
  if (!nul) return 48;
  if (!fbout) return 49;
  if (!check_main) return 50;
  if (!strct) return 51;
  if (!gstruct) return 52;
  if (!abstrct) return 53;
  if (!fbsuper) return 54;
  if (!pool) return 55;

  if (basic() != 67) return 1;
  if (copy() != 60) return 2;
  if (array() != 186) return 3;
  if (nested() != 22) return 4;
  if (global() != 19) return 5;
  if (rec() != 55) return 6;
  if (fb() != 33) return 7;
  if (param() != 58) return 8;
  if (nul() != 33) return 9;
  if (strct() != 25) return 12;
  if (gstruct() != 11) return 13;
  if (abstrct() != 25) return 14;
  if (fbsuper() != 12) return 15;
  if (pool() != 12) return 16;
  if (fbout() != 9) return 10;
  if (check_main() != 0) return 11;

  if (stc_host_cycle() != 0) return 20;
  if (check_main() != 22) return 22;
  if (stc_host_shutdown() != 0) return 21;
  return 0;
}
