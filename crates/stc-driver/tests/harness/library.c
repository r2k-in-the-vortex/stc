/* M4.5 library-mode harness: load the module through the embedding host. */

typedef unsigned char u8;
typedef unsigned long long u64;

typedef void (*void_fn)(void);
typedef int (*int_fn)(void);
typedef int (*int2_fn)(int, int);
typedef int (*int1_fn)(int);
typedef unsigned char (*bool1_fn)(int);
typedef double (*double1_fn)(double);
typedef long long (*ll_fn)(void);
typedef int (*buf_fn)(void*);

extern int stc_host_load_module(const char* path, int mode);
extern int stc_host_cycle(void);
extern int stc_host_shutdown(void);
extern void* stc_host_symbol(const char* name);
extern int stc_abi_version(void);
extern const char* stc_metadata_json(void);
extern int stc_host_export_manifest(char* buffer, long long* len);

static int contains(const char* text, const char* needle) {
  while (*text != 0) {
    const char* left = text;
    const char* right = needle;
    while (*right != 0 && *left == *right) {
      ++left;
      ++right;
    }
    if (*right == 0) return 1;
    ++text;
  }
  return 0;
}

int main(int argc, char** argv) {
  if (argc < 2) return 11;
  if (stc_abi_version() != 2) return 1;
  if (stc_host_load_module(argv[1], 0) != 0) return 2;
  const char* metadata = stc_metadata_json();
  if (metadata == 0) return 12;
  if (!contains(metadata, "\"profile\"")) return 3;
  if (!contains(metadata, "stc-2013-oop-utf8-v1")) return 4;

  int2_fn test_add = (int2_fn)stc_host_symbol("stc_export_TESTADD");
  int1_fn test_fact = (int1_fn)stc_host_symbol("TestFact");
  int1_fn test_control = (int1_fn)stc_host_symbol("stc_export_TESTCONTROL");
  bool1_fn is_pos = (bool1_fn)stc_host_symbol("IsPos");
  double1_fn half = (double1_fn)stc_host_symbol("Half");
  if (!test_add || !test_fact || !test_control || !is_pos || !half) return 13;
  if (test_add(2, 3) != 5) return 5;
  if (test_fact(5) != 120) return 6;
  if (test_control(10) != 100) return 7;
  if (is_pos(3) != 1) return 8;
  if (is_pos(-1) != 0) return 9;
  if (half(3.0) != 1.5) return 10;
  long long manifest_len = 0;
  if (stc_host_export_manifest(0, &manifest_len) != 0 || manifest_len <= 0) return 16;
  static char manifest[65536];
  if (manifest_len > (long long)sizeof(manifest)) return 17;
  if (stc_host_export_manifest(manifest, &manifest_len) != 0) return 18;
  if (!contains(manifest, "build_id")) return 19;
  if (stc_host_cycle() != 0) return 14;
  if (stc_host_shutdown() != 0) return 15;
  return 0;
}
