; Cross-target object-emission smoke module (M4). No target datalayout or triple:
; clang selects them from --target.
source_filename = "smoke"

define i32 @smoke_add(i32 %a, i32 %b) {
entry:
  %sum = add i32 %a, %b
  ret i32 %sum
}
