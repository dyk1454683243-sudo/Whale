#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="${ROOT_DIR}/tests/nasm-diff"
OUT_DIR="${ROOT_DIR}/target/nasm-diff"
WHALE_BIN=(cargo run -q -- asm --amd64)

if ! command -v nasm >/dev/null 2>&1; then
  echo "error: 'nasm' is required" >&2
  exit 1
fi
if ! command -v readelf >/dev/null 2>&1; then
  echo "error: 'readelf' is required" >&2
  exit 1
fi
if ! command -v objcopy >/dev/null 2>&1; then
  echo "error: 'objcopy' is required" >&2
  exit 1
fi

mkdir -p "${OUT_DIR}"
rm -f "${OUT_DIR}"/*

normalize_relocs() {
  local obj="$1"
  readelf -Wr "${obj}" \
    | awk '
      $3 ~ /^R_X86_64_/ {
        add = 0
        if (NF >= 7) {
          if ($6 == "-") add = -$7
          else if ($6 == "+") add = $7
          else add = $6
        }
        print $1, $3, $5, add
      }
    ' \
    | sort
}

normalize_global_symbols() {
  local obj="$1"
  readelf -Ws "${obj}" \
    | awk '
      $5 == "GLOBAL" {
        st = ($7 == "UND") ? "UND" : "DEF"
        print $8, st
      }
    ' \
    | sort
}

check_no_readelf_warning() {
  local obj="$1"
  if readelf -Wa "${obj}" 2>&1 | grep -q "^readelf: Warning:"; then
    echo "readelf warning detected: ${obj}" >&2
    readelf -Wa "${obj}" 2>&1 | grep "^readelf: Warning:" >&2
    return 1
  fi
}

compare_section_bytes() {
  local base="$1"
  local section="$2"
  local nasm_obj="$3"
  local whale_obj="$4"
  local nasm_bin="${OUT_DIR}/${base}.nasm${section}.bin"
  local whale_bin="${OUT_DIR}/${base}.whale${section}.bin"

  local nasm_has=0
  local whale_has=0
  rm -f "${nasm_bin}" "${whale_bin}"
  objcopy --dump-section "${section}=${nasm_bin}" "${nasm_obj}" >/dev/null 2>&1 || true
  objcopy --dump-section "${section}=${whale_bin}" "${whale_obj}" >/dev/null 2>&1 || true

  if [[ -f "${nasm_bin}" ]]; then
    nasm_has=1
  else
    : > "${nasm_bin}"
  fi
  if [[ -f "${whale_bin}" ]]; then
    whale_has=1
  else
    : > "${whale_bin}"
  fi

  if [[ "${nasm_has}" -ne "${whale_has}" ]]; then
    # Treat "missing section" and "present but empty section" as equivalent.
    if [[ "${nasm_has}" -eq 0 && ! -s "${whale_bin}" ]]; then
      return 0
    fi
    if [[ "${whale_has}" -eq 0 && ! -s "${nasm_bin}" ]]; then
      return 0
    fi
    echo "section mismatch for ${base}: ${section} existence/size differs" >&2
    return 1
  fi
  if [[ "${nasm_has}" -eq 0 ]]; then
    return 0
  fi

  if ! cmp -s "${nasm_bin}" "${whale_bin}"; then
    echo "byte mismatch for ${base} ${section}" >&2
    diff -u <(xxd -g1 "${nasm_bin}") <(xxd -g1 "${whale_bin}") >&2 || true
    return 1
  fi
}

ok=0
fail=0

for asm in "${TEST_DIR}"/*.asm; do
  base="$(basename "${asm}" .asm)"
  nasm_obj="${OUT_DIR}/${base}.nasm.o"
  whale_obj="${OUT_DIR}/${base}.whale.o"

  if ! nasm -f elf64 "${asm}" -o "${nasm_obj}" >/dev/null 2>&1; then
    echo "FAIL ${base}: nasm assemble failed" >&2
    fail=$((fail + 1))
    continue
  fi

  if ! "${WHALE_BIN[@]}" "${asm}" -o "${whale_obj}" >/dev/null 2>&1; then
    echo "FAIL ${base}: whale assemble failed" >&2
    fail=$((fail + 1))
    continue
  fi

  case_ok=1

  if ! check_no_readelf_warning "${whale_obj}"; then
    case_ok=0
  fi
  if ! compare_section_bytes "${base}" ".text" "${nasm_obj}" "${whale_obj}"; then
    case_ok=0
  fi
  if ! compare_section_bytes "${base}" ".data" "${nasm_obj}" "${whale_obj}"; then
    case_ok=0
  fi
  if ! compare_section_bytes "${base}" ".rodata" "${nasm_obj}" "${whale_obj}"; then
    case_ok=0
  fi

  normalize_relocs "${nasm_obj}" > "${OUT_DIR}/${base}.nasm.relocs"
  normalize_relocs "${whale_obj}" > "${OUT_DIR}/${base}.whale.relocs"
  if ! diff -u "${OUT_DIR}/${base}.nasm.relocs" "${OUT_DIR}/${base}.whale.relocs" >/dev/null; then
    echo "relocation mismatch for ${base}" >&2
    diff -u "${OUT_DIR}/${base}.nasm.relocs" "${OUT_DIR}/${base}.whale.relocs" >&2 || true
    case_ok=0
  fi

  normalize_global_symbols "${nasm_obj}" > "${OUT_DIR}/${base}.nasm.globals"
  normalize_global_symbols "${whale_obj}" > "${OUT_DIR}/${base}.whale.globals"
  if ! diff -u "${OUT_DIR}/${base}.nasm.globals" "${OUT_DIR}/${base}.whale.globals" >/dev/null; then
    echo "global/undefined symbol mismatch for ${base}" >&2
    diff -u "${OUT_DIR}/${base}.nasm.globals" "${OUT_DIR}/${base}.whale.globals" >&2 || true
    case_ok=0
  fi

  if [[ "${case_ok}" -eq 1 ]]; then
    echo "OK   ${base}"
    ok=$((ok + 1))
  else
    echo "FAIL ${base}"
    fail=$((fail + 1))
  fi
done

echo "SUMMARY ok=${ok} fail=${fail}"
if [[ "${fail}" -ne 0 ]]; then
  exit 1
fi
