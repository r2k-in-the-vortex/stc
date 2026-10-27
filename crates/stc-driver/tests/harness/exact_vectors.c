/* M6 exact-vector harness: executes committed vectors through generated code.
 *
 * Usage:
 *   exact_vectors <module> <cases-file>
 *   exact_vectors <module> --trap
 *
 * The case file is generated from the committed JSON vector files and contains
 * one line per executable case:
 *
 *   B <symbol> <a> <b> <expected>       binary operation
 *   T <symbol> <g> <a> <b> <expected>   ternary (SEL)
 *   S <symbol> <expected>               nullary (string length)
 *
 * All results are compared as 64-bit bit patterns. Exported wrappers return
 * LINT or LWORD so the upper bits are defined.
 *
 * The harness declares the few C runtime functions it uses instead of
 * including standard headers, matching the other harness samples: the CI
 * clang does not discover the MSVC CRT headers. It also parses the case file
 * in memory because some CRT stdio entry points are header-inline only.
 */

typedef unsigned long long u64;

extern void* fopen(const char*, const char*);
extern u64 fread(void*, u64, u64, void*);
extern int fseek(void*, long, int);
extern long ftell(void*);
extern int fclose(void*);
extern int printf(const char*, ...);
extern int strcmp(const char*, const char*);

typedef u64 (*bin_fn)(u64, u64);
typedef u64 (*tern_fn)(u64, u64, u64);
typedef u64 (*null_fn)(void);

extern int stc_host_load_module(const char* path, int mode);
extern int stc_host_cycle(void);
extern int stc_host_shutdown(void);
extern void* stc_host_symbol(const char* name);

static char case_buffer[1 << 20];

static int is_space(char c) {
  return c == ' ' || c == '\t' || c == '\r' || c == '\n';
}

static char* skip_space(char* p) {
  while (*p != 0 && is_space(*p)) {
    ++p;
  }
  return p;
}

static char* read_token(char* p, char* out, int capacity) {
  int length = 0;
  p = skip_space(p);
  while (*p != 0 && !is_space(*p) && length < capacity - 1) {
    out[length++] = *p++;
  }
  out[length] = 0;
  return p;
}

static char* read_u64(char* p, u64* out) {
  u64 value = 0;
  p = skip_space(p);
  while (*p >= '0' && *p <= '9') {
    value = value * 10 + (u64)(*p - '0');
    ++p;
  }
  *out = value;
  return p;
}

static int run_cases(const char* path) {
  void* file = fopen(path, "rb");
  if (file == 0) {
    printf("cannot open case file %s\n", path);
    return 12;
  }
  if (fseek(file, 0, 2) != 0) {
    return 17;
  }
  long size = ftell(file);
  if (size <= 0 || size >= (long)sizeof(case_buffer)) {
    fclose(file);
    return 17;
  }
  if (fseek(file, 0, 0) != 0) {
    fclose(file);
    return 17;
  }
  u64 read = fread(case_buffer, 1, (u64)size, file);
  fclose(file);
  case_buffer[read] = 0;

  char* p = case_buffer;
  int total = 0;
  int failures = 0;
  while (1) {
    char kind[16];
    char symbol[128];
    u64 a = 0;
    u64 b = 0;
    u64 c = 0;
    u64 expected = 0;
    u64 actual = 0;
    void* entry = 0;
    p = read_token(p, kind, (int)sizeof(kind));
    if (kind[0] == 0) {
      break;
    }
    p = read_token(p, symbol, (int)sizeof(symbol));
    if (strcmp(kind, "B") == 0) {
      p = read_u64(p, &a);
      p = read_u64(p, &b);
      p = read_u64(p, &expected);
      entry = stc_host_symbol(symbol);
      if (entry == 0) {
        printf("missing symbol %s\n", symbol);
        return 14;
      }
      actual = ((bin_fn)entry)(a, b);
    } else if (strcmp(kind, "T") == 0) {
      p = read_u64(p, &a);
      p = read_u64(p, &b);
      p = read_u64(p, &c);
      p = read_u64(p, &expected);
      entry = stc_host_symbol(symbol);
      if (entry == 0) {
        printf("missing symbol %s\n", symbol);
        return 14;
      }
      actual = ((tern_fn)entry)(a, b, c);
    } else if (strcmp(kind, "S") == 0) {
      p = read_u64(p, &expected);
      entry = stc_host_symbol(symbol);
      if (entry == 0) {
        printf("missing symbol %s\n", symbol);
        return 14;
      }
      actual = ((null_fn)entry)();
    } else {
      printf("unknown case kind %s\n", kind);
      return 15;
    }
    ++total;
    if (actual != expected) {
      ++failures;
      printf("%s: got 0x%llx expected 0x%llx\n", symbol, actual, expected);
    }
  }
  if (failures != 0) {
    printf("%d of %d exact vector cases failed\n", failures, total);
    return 1;
  }
  printf("executed %d exact vector cases\n", total);
  return 0;
}

int main(int argc, char** argv) {
  if (argc < 3) {
    return 11;
  }
  if (stc_host_load_module(argv[1], 0) != 0) {
    return 2;
  }
  if (strcmp(argv[2], "--trap") == 0) {
    int result = stc_host_cycle();
    if (result == 0) {
      printf("checked arithmetic did not trap\n");
      return 1;
    }
    return 0;
  }
  int status = run_cases(argv[2]);
  if (status != 0) {
    return status;
  }
  if (stc_host_shutdown() != 0) {
    return 16;
  }
  return 0;
}
