/* M4 library-mode harness: no standard headers, so clang needs no SDK includes. */

typedef unsigned char u8;
typedef unsigned long long u64;

extern void stc_init(void);
extern void stc_cycle(void);
extern void stc_shutdown(void);
extern int stc_abi_version(void);
extern const char* stc_metadata_json(void);
extern int stc_export_TESTADD(int a, int b);
extern int stc_export_TESTCONTROL(int seed);
extern int TestFact(int n);
extern unsigned char IsPos(int x);
extern double Half(double x);

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

int main(void) {
  if (stc_abi_version() != 1) return 1;
  const char* metadata = stc_metadata_json();
  if (metadata == 0) return 2;
  if (!contains(metadata, "\"profile\"")) return 3;
  if (!contains(metadata, "stc-2013-oop-utf8-v1")) return 4;

  stc_init();
  if (stc_export_TESTADD(2, 3) != 5) return 5;
  if (TestFact(5) != 120) return 6;
  if (stc_export_TESTCONTROL(10) != 100) return 7;
  if (IsPos(3) != 1) return 8;
  if (IsPos(-1) != 0) return 9;
  if (Half(3.0) != 1.5) return 10;
  stc_cycle();
  stc_shutdown();
  return 0;
}
