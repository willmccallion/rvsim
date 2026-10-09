// Replays an rvsim commit log on spike, one instruction at a time, and
// reports the first instruction whose architectural effects differ: its PC,
// privilege mode, destination register, the CSR or fflags it wrote, or the
// memory it read or wrote. Values that legitimately differ between the two
// models (counters, timers, implementation IDs, device registers and
// interrupt arrival) are taken from rvsim's log and given to spike.

#include <riscv/cfg.h>
#include <riscv/disasm.h>
#include <riscv/encoding.h>
#include <riscv/mmu.h>
#include <riscv/processor.h>
#include <riscv/simif.h>

#include <elf.h>

#include <cinttypes>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <deque>
#include <fstream>
#include <iostream>
#include <map>
#include <optional>
#include <sstream>
#include <stdexcept>
#include <string>
#include <utility>
#include <vector>

namespace {

constexpr uint64_t interrupt_bit = 1ULL << 63;
// misa.B may be clear on a hart with Zba, Zbb and Zbs; spike sets it.
constexpr uint64_t misa_b = 1ULL << ('B' - 'A');
constexpr size_t context_lines = 12;

struct MemAccess {
  uint64_t vaddr;
  uint64_t paddr;
  unsigned bytes;
  uint64_t value;
};

enum class RegFile { integer, floating };

struct RegWrite {
  RegFile file;
  unsigned index;
  uint64_t value;
};

struct CsrWrite {
  unsigned addr;
  uint64_t value;
};

struct Retired {
  uint64_t pc;
  uint32_t inst;
  unsigned privilege;
  std::optional<RegWrite> destination;
  std::vector<CsrWrite> csrs;
  std::optional<MemAccess> load;
  std::optional<MemAccess> store;
  bool vector;
};

struct Trap {
  uint64_t cause;
  uint64_t epc;
  uint64_t tval;
};

struct ResetState {
  uint64_t pc = 0;
  unsigned privilege = PRV_M;
  std::map<unsigned, uint64_t> xregs;
  std::map<unsigned, uint64_t> fregs;
  std::vector<CsrWrite> csrs;
};

struct Record {
  enum class Kind { retired, trap } kind;
  Retired retired;
  Trap trap;
  std::string text;
};

uint64_t parse_hex(const std::string& token) {
  return std::stoull(token, nullptr, 16);
}

MemAccess parse_mem(std::istringstream& fields) {
  std::string vaddr, paddr, bytes, value;
  fields >> vaddr >> paddr >> bytes >> value;
  return MemAccess{parse_hex(vaddr), parse_hex(paddr), static_cast<unsigned>(std::stoul(bytes)),
                   parse_hex(value)};
}

class CommitLog {
 public:
  explicit CommitLog(const std::string& path) : in_(path) {
    if (!in_) throw std::runtime_error("cannot open commit log " + path);
  }

  ResetState read_reset() {
    ResetState reset;
    std::string line;
    while (peek_line(line) && line.rfind("core   0: reset ", 0) == 0) {
      consume_line();
      std::istringstream fields(line.substr(16));
      std::string what, value;
      fields >> what >> value;
      if (what == "pc") {
        reset.pc = parse_hex(value);
        std::string priv_word;
        unsigned privilege = PRV_M;
        fields >> priv_word >> privilege;
        reset.privilege = privilege;
      } else if (what[0] == 'x') {
        reset.xregs[std::stoul(what.substr(1))] = parse_hex(value);
      } else if (what[0] == 'f') {
        reset.fregs[std::stoul(what.substr(1))] = parse_hex(value);
      } else if (what[0] == 'c') {
        reset.csrs.push_back(CsrWrite{static_cast<unsigned>(std::stoul(what.substr(1), nullptr, 16)),
                                      parse_hex(value)});
      }
    }
    return reset;
  }

  std::optional<Record> next() {
    std::string line;
    while (peek_line(line)) {
      consume_line();
      if (line.rfind("core   0: ", 0) != 0) continue;
      remember(line);
      try {
        return parse_record(line);
      } catch (const std::logic_error&) {
        throw std::runtime_error("malformed commit log line " + std::to_string(lines_read_) + ": " + line);
      }
    }
    return std::nullopt;
  }

  std::optional<Record> peek() {
    if (!lookahead_) lookahead_ = next();
    return lookahead_;
  }

  std::optional<Record> take() {
    if (lookahead_) {
      auto record = lookahead_;
      lookahead_.reset();
      return record;
    }
    return next();
  }

  const std::deque<std::string>& recent() const { return recent_; }
  uint64_t lines_read() const { return lines_read_; }

 private:
  static Record parse_record(const std::string& line) {
    std::istringstream fields(line.substr(10));
    std::string first;
    fields >> first;
    Record record{};
    record.text = line;
    if (first == "trap") {
      std::string cause, epc, tval;
      fields >> cause >> epc >> tval;
      record.kind = Record::Kind::trap;
      record.trap = Trap{parse_hex(cause), parse_hex(epc), parse_hex(tval)};
      return record;
    }
    record.kind = Record::Kind::retired;
    Retired& retired = record.retired;
    retired.pc = parse_hex(first);
    std::string inst;
    fields >> inst;
    retired.inst = static_cast<uint32_t>(parse_hex(inst.substr(1, inst.size() - 2)));
    std::string token;
    while (fields >> token) {
      if (token == "priv") {
        fields >> retired.privilege;
      } else if (token == "load") {
        retired.load = parse_mem(fields);
      } else if (token == "store") {
        retired.store = parse_mem(fields);
      } else if (token == "vec") {
        retired.vector = true;
      } else {
        std::string value;
        fields >> value;
        unsigned index = static_cast<unsigned>(std::stoul(token.substr(1), nullptr, token[0] == 'c' ? 16 : 10));
        if (token[0] == 'x') {
          retired.destination = RegWrite{RegFile::integer, index, parse_hex(value)};
        } else if (token[0] == 'f') {
          retired.destination = RegWrite{RegFile::floating, index, parse_hex(value)};
        } else if (token[0] == 'c') {
          retired.csrs.push_back(CsrWrite{index, parse_hex(value)});
        }
      }
    }
    return record;
  }

  bool peek_line(std::string& line) {
    if (!pending_line_) {
      std::string next_line;
      if (!std::getline(in_, next_line)) return false;
      pending_line_ = next_line;
    }
    line = *pending_line_;
    return true;
  }

  void consume_line() {
    pending_line_.reset();
    ++lines_read_;
  }

  void remember(const std::string& line) {
    recent_.push_back(line);
    if (recent_.size() > context_lines) recent_.pop_front();
  }

  std::ifstream in_;
  std::optional<std::string> pending_line_;
  std::optional<Record> lookahead_;
  std::deque<std::string> recent_;
  uint64_t lines_read_ = 0;
};

class Divergence : public std::runtime_error {
 public:
  using std::runtime_error::runtime_error;
};

std::string hex(uint64_t value) {
  char buffer[24];
  std::snprintf(buffer, sizeof buffer, "0x%016" PRIx64, value);
  return buffer;
}

uint64_t width_mask(unsigned bytes) {
  return bytes >= 8 ? ~0ULL : (1ULL << (8 * bytes)) - 1;
}

// Spike's view of the machine: RAM it owns, and every other address served
// from the rvsim access the current instruction logged.
class Platform : public simif_t {
 public:
  Platform(const cfg_t& cfg, uint64_t ram_base, uint64_t ram_size)
      : cfg_(cfg), ram_base_(ram_base), ram_(ram_size, 0) {}

  char* addr_to_mem(reg_t paddr) override {
    if (paddr < ram_base_ || paddr - ram_base_ >= ram_.size()) return nullptr;
    return &ram_[paddr - ram_base_];
  }

  bool mmio_fetch(reg_t, size_t, uint8_t*) override { return false; }

  bool mmio_load(reg_t paddr, size_t len, uint8_t* bytes) override {
    if (!expected_load_ || expected_load_->paddr != paddr || expected_load_->bytes != len) {
      unexpected_device_access_ = "spike read device " + hex(paddr) + " (" + std::to_string(len) +
                                  " bytes), rvsim logged " +
                                  (expected_load_ ? "a load of " + hex(expected_load_->paddr) : "no load");
      return false;
    }
    std::memcpy(bytes, &expected_load_->value, len);
    expected_load_.reset();
    ++device_reads_;
    return true;
  }

  bool mmio_store(reg_t paddr, size_t len, const uint8_t* bytes) override {
    uint64_t value = 0;
    std::memcpy(&value, bytes, std::min<size_t>(len, sizeof value));
    device_writes_.push_back(MemAccess{0, paddr, static_cast<unsigned>(len), value});
    return true;
  }

  void proc_reset(unsigned) override {}
  const cfg_t& get_cfg() const override { return cfg_; }
  const std::map<size_t, processor_t*>& get_harts() const override { return harts_; }
  const char* get_symbol(uint64_t) override { return nullptr; }

  void add_hart(processor_t* hart) { harts_[0] = hart; }

  void write_ram(uint64_t paddr, const char* data, size_t size) {
    char* host = addr_to_mem(paddr);
    if (!host || paddr - ram_base_ + size > ram_.size())
      throw std::runtime_error("image segment at " + hex(paddr) + " lies outside RAM");
    std::memcpy(host, data, size);
  }

  void expect_load(std::optional<MemAccess> load) { expected_load_ = load; }
  std::vector<MemAccess> take_device_writes() { return std::exchange(device_writes_, {}); }
  std::optional<std::string> take_unexpected_access() { return std::exchange(unexpected_device_access_, std::nullopt); }
  uint64_t device_reads() const { return device_reads_; }

 private:
  const cfg_t& cfg_;
  uint64_t ram_base_;
  std::vector<char> ram_;
  std::map<size_t, processor_t*> harts_;
  std::optional<MemAccess> expected_load_;
  std::vector<MemAccess> device_writes_;
  std::optional<std::string> unexpected_device_access_;
  uint64_t device_reads_ = 0;
};

void load_elf(Platform& platform, const std::string& path) {
  std::ifstream file(path, std::ios::binary);
  if (!file) throw std::runtime_error("cannot open " + path);
  std::vector<char> image((std::istreambuf_iterator<char>(file)), std::istreambuf_iterator<char>());
  if (image.size() < sizeof(Elf64_Ehdr) || std::memcmp(image.data(), ELFMAG, SELFMAG) != 0 ||
      image[EI_CLASS] != ELFCLASS64)
    throw std::runtime_error(path + " is not a 64-bit ELF");
  const auto* header = reinterpret_cast<const Elf64_Ehdr*>(image.data());
  for (unsigned i = 0; i < header->e_phnum; ++i) {
    const auto* segment = reinterpret_cast<const Elf64_Phdr*>(image.data() + header->e_phoff +
                                                              i * header->e_phentsize);
    if (segment->p_type != PT_LOAD || segment->p_memsz == 0) continue;
    std::vector<char> contents(segment->p_memsz, 0);
    std::memcpy(contents.data(), image.data() + segment->p_offset, segment->p_filesz);
    platform.write_ram(segment->p_paddr, contents.data(), contents.size());
  }
}

void load_raw(Platform& platform, const std::string& spec) {
  auto at = spec.rfind('@');
  if (at == std::string::npos) throw std::runtime_error("--load wants FILE@ADDR, got " + spec);
  std::ifstream file(spec.substr(0, at), std::ios::binary);
  if (!file) throw std::runtime_error("cannot open " + spec.substr(0, at));
  std::vector<char> contents((std::istreambuf_iterator<char>(file)), std::istreambuf_iterator<char>());
  platform.write_ram(parse_hex(spec.substr(at + 1)), contents.data(), contents.size());
}

bool is_store_conditional(insn_t insn) {
  return (insn.bits() & 0x7f) == 0x2f && (insn.bits() >> 27) == 0b00011;
}

bool is_csr_access(insn_t insn) {
  return (insn.bits() & 0x7f) == 0x73 && insn.rm() != 0 && insn.rm() != 4;
}

// The bits a CSR instruction that writes mip or sip sets (`sets`) or
// clears (`clears`), read before it executes.
struct InterruptPendingWrite {
  reg_t sets;
  reg_t clears;
};

std::optional<InterruptPendingWrite> pending_write(insn_t insn, const state_t& state) {
  if (!is_csr_access(insn) || (insn.csr() != CSR_MIP && insn.csr() != CSR_SIP)) return std::nullopt;
  reg_t operand = insn.rm() >= 4 ? insn.rs1() : state.XPR[insn.rs1()];
  switch (insn.rm() & 3) {
    case 1:
      return InterruptPendingWrite{operand, ~operand};
    case 2:
      return InterruptPendingWrite{operand, 0};
    default:
      return InterruptPendingWrite{0, operand};
  }
}

// CSRs rvsim implements and spike does not: tcontrol is optional in
// Sdtrig. Their accesses are taken from rvsim's log.
bool is_rvsim_only(unsigned csr) {
  return csr == CSR_TCONTROL;
}

// The bits of a CSR whose WARL behaviour the two models legitimately
// choose differently: spike hardwires medeleg's misaligned-fetch bit when C
// is present, rvsim lets it be delegated.
reg_t implementation_defined_bits(unsigned csr) {
  return csr == CSR_MEDELEG ? 1ULL << CAUSE_MISALIGNED_FETCH : 0;
}

// CSRs whose value is the implementation's or the clock's, not the ISA's;
// misa is checked at reset.
bool is_model_specific(unsigned csr) {
  return (csr >= CSR_CYCLE && csr <= CSR_HPMCOUNTER31) ||
         (csr >= CSR_MCYCLE && csr <= CSR_MHPMCOUNTER31) || csr == CSR_MIP || csr == CSR_SIP ||
         csr == CSR_MVENDORID || csr == CSR_MARCHID || csr == CSR_MIMPID || csr == CSR_MCONFIGPTR ||
         csr == CSR_MISA;
}

class Lockstep {
 public:
  Lockstep(processor_t& hart, Platform& platform, CommitLog& log, uint64_t& commits)
      : hart_(hart), state_(*hart.get_state()), platform_(platform), log_(log), commits_(commits) {}

  void apply_reset(const ResetState& reset) {
    state_.pc = reset.pc;
    hart_.set_privilege(reset.privilege, false);
    for (auto [index, value] : reset.xregs) state_.XPR.write(index, value);
    for (auto [index, value] : reset.fregs) state_.FPR.write(index, boxed(value));
    for (auto [addr, value] : reset.csrs) {
      if (addr == CSR_MISA) {
        uint64_t spike_misa = read_csr(CSR_MISA);
        if ((spike_misa & ~misa_b) != (value & ~misa_b))
          throw std::runtime_error("misa differs: rvsim " + hex(value) + ", spike " + hex(spike_misa) +
                                   "; spike lacks an extension rvsim has");
        continue;
      }
      hart_.put_csr(addr, value);
    }
  }

  void run() {
    while (auto record = log_.take()) {
      if (record->kind == Record::Kind::trap) {
        take_logged_trap(record->trap);
      } else {
        retire(record->retired);
      }
    }
  }

  uint64_t instructions() const { return instructions_; }
  uint64_t traps() const { return traps_; }
  uint64_t injected_csr_reads() const { return injected_csr_reads_; }
  uint64_t emulated_csr_accesses() const { return emulated_csr_accesses_; }
  uint64_t failed_store_conditionals() const { return failed_store_conditionals_; }

 private:
  enum class Outcome { retired, trapped };

  static freg_t boxed(uint64_t value) {
    freg_t reg;
    reg.v[0] = value;
    reg.v[1] = ~0ULL;
    return reg;
  }

  // Steps the instruction at the PC without letting spike take an interrupt
  // first: rvsim's log says when one is taken. The interrupts software left
  // pending stay pending unless the instruction clears them.
  Outcome step_instruction(insn_t insn) {
    reg_t pending = state_.mip->read();
    auto write = pending_write(insn, state_);
    state_.mip->backdoor_write_with_mask(pending, 0);
    Outcome outcome = step();
    reg_t keep = pending & ~(write ? write->clears : 0);
    state_.mip->backdoor_write_with_mask(keep, keep);
    return outcome;
  }

  Outcome step() {
    for (int attempt = 0; attempt < 4; ++attempt) {
      uint64_t before = commits_;
      hart_.step(1);
      if (commits_ != before) return Outcome::retired;
      if (!state_.serialized) return Outcome::trapped;
    }
    throw Divergence("spike made no progress at " + hex(state_.pc));
  }

  void retire(const Retired& expected) {
    if (state_.pc != expected.pc)
      throw Divergence("PC differs: rvsim retired " + hex(expected.pc) + ", spike is at " + hex(state_.pc));
    insn_t insn = fetch(expected.pc);
    if (insn.length() == 4 && static_cast<uint32_t>(insn.bits()) != expected.inst)
      throw Divergence("instruction bits differ at " + hex(expected.pc));

    if (is_csr_access(insn) && is_rvsim_only(insn.csr())) {
      take_rvsim_only_csr_access(insn, expected);
      return;
    }

    hart_.clear_waiting_for_interrupt();
    follow_store_conditional_outcome(insn, expected);
    platform_.expect_load(expected.load && is_device(expected.load->paddr) ? expected.load : std::nullopt);
    Outcome outcome = step_instruction(insn);
    if (auto unexpected = platform_.take_unexpected_access()) throw Divergence(*unexpected);

    if (outcome == Outcome::trapped) {
      auto next = log_.peek();
      if (next && next->kind != Record::Kind::trap)
        throw Divergence("spike trapped (cause " + hex(trap_cause()) + ") where rvsim retired the instruction");
      took_exception_ = true;
      return;
    }
    ++instructions_;
    if (state_.last_inst_priv != expected.privilege)
      throw Divergence("privilege differs: rvsim " + std::to_string(expected.privilege) + ", spike " +
                       std::to_string(state_.last_inst_priv));
    inject_model_specific_read(insn, expected);
    compare_registers(expected);
    compare_csrs(expected, is_csr_access(insn));
    if (!expected.vector) compare_memory(expected);
  }

  void take_logged_trap(const Trap& expected) {
    ++traps_;
    bool interrupt = (expected.cause & interrupt_bit) != 0;
    uint64_t code = expected.cause & ~interrupt_bit;
    if (interrupt) {
      reg_t bit = 1ULL << code;
      bool software_pending = (state_.mip->read() & bit) != 0;
      state_.mip->backdoor_write_with_mask(bit, bit);
      Outcome outcome = step();
      if (!software_pending) state_.mip->backdoor_write_with_mask(bit, 0);
      if (outcome != Outcome::trapped) throw Divergence("spike did not take interrupt " + hex(expected.cause));
    } else if (!std::exchange(took_exception_, false)) {
      // A fetch fault has no instruction line, so spike has yet to step into it.
      if (step() != Outcome::trapped)
        throw Divergence("spike retired where rvsim took fetch fault " + hex(expected.cause));
    }
    uint64_t cause = trap_cause();
    if (cause != expected.cause || trap_epc() != expected.epc || (!interrupt && trap_tval() != expected.tval))
      throw Divergence("trap differs: rvsim cause " + hex(expected.cause) + " epc " + hex(expected.epc) +
                       " tval " + hex(expected.tval) + ", spike cause " + hex(cause) + " epc " +
                       hex(trap_epc()) + " tval " + hex(trap_tval()));
  }

  // A reservation may be lost for reasons the ISA leaves to the
  // implementation, such as the hart's own store to the reserved address,
  // which rvsim's breaks and spike's does not. When rvsim's SC failed,
  // spike's fails too.
  void follow_store_conditional_outcome(insn_t insn, const Retired& expected) {
    if (!is_store_conditional(insn) || expected.store) return;
    hart_.get_mmu()->yield_load_reservation();
    ++failed_store_conditionals_;
  }

  // Retires a CSR instruction for a CSR spike lacks as rvsim did: its
  // result is rvsim's, and nothing else changes.
  void take_rvsim_only_csr_access(insn_t insn, const Retired& expected) {
    if (expected.destination && insn.rd() != 0) state_.XPR.write(insn.rd(), expected.destination->value);
    state_.pc = expected.pc + insn.length();
    ++instructions_;
    ++emulated_csr_accesses_;
  }

  void inject_model_specific_read(insn_t insn, const Retired& expected) {
    if (!is_csr_access(insn) || insn.rd() == 0) return;
    if (!expected.destination) throw Divergence("rvsim logged no result for a CSR read at " + hex(expected.pc));
    uint64_t spike_value = state_.XPR[insn.rd()];
    uint64_t rvsim_value = expected.destination->value;
    reg_t free_bits = implementation_defined_bits(insn.csr());
    bool differs_only_by_choice = free_bits != 0 && ((spike_value ^ rvsim_value) & ~free_bits) == 0;
    if (!is_model_specific(insn.csr()) && !differs_only_by_choice) return;
    state_.XPR.write(insn.rd(), rvsim_value);
    ++injected_csr_reads_;
  }

  void compare_registers(const Retired& expected) {
    std::optional<RegWrite> spike_write;
    for (const auto& [key, value] : state_.log_reg_write) {
      unsigned index = static_cast<unsigned>(key >> 4);
      switch (key & 0xf) {
        case 0:
          if (index != 0) spike_write = RegWrite{RegFile::integer, index, state_.XPR[index]};
          break;
        case 1:
          spike_write = RegWrite{RegFile::floating, index, state_.FPR[index].v[0]};
          break;
        default:
          break;
      }
    }
    if (!spike_write && !expected.destination) return;
    if (!spike_write || !expected.destination || spike_write->file != expected.destination->file ||
        spike_write->index != expected.destination->index || spike_write->value != expected.destination->value)
      throw Divergence("destination differs: rvsim " + describe(expected.destination) + ", spike " +
                       describe(spike_write));
  }

  void compare_csrs(const Retired& expected, bool csr_instruction) {
    bool rvsim_logged_fflags = false;
    for (auto [addr, value] : expected.csrs) {
      rvsim_logged_fflags |= addr == CSR_FFLAGS;
      if (is_model_specific(addr)) continue;
      uint64_t spike_value = read_csr(addr);
      if ((spike_value ^ value) & ~implementation_defined_bits(addr))
        throw Divergence("CSR " + hex(addr) + " differs: rvsim " + hex(value) + ", spike " + hex(spike_value));
    }
    bool spike_raised_fp_flags = !csr_instruction && state_.log_reg_write.count((CSR_FFLAGS << 4) | 4) != 0;
    if (spike_raised_fp_flags && !rvsim_logged_fflags && !expected.vector)
      throw Divergence("spike raised FP flags " + hex(hart_.get_csr(CSR_FFLAGS)) + ", rvsim raised none");
  }

  void compare_memory(const Retired& expected) {
    compare_access("load", expected.load, state_.log_mem_read);
    compare_access("store", expected.store, state_.log_mem_write);
    for (const MemAccess& write : platform_.take_device_writes()) {
      if (!expected.store || expected.store->paddr != write.paddr ||
          (expected.store->value & width_mask(write.bytes)) != (write.value & width_mask(write.bytes)))
        throw Divergence("device write differs: spike wrote " + hex(write.value) + " to " + hex(write.paddr) +
                         ", rvsim logged " + (expected.store ? hex(expected.store->value) + " to " + hex(expected.store->paddr) : "none"));
    }
  }

  static void compare_access(const char* kind, const std::optional<MemAccess>& expected,
                             const commit_log_mem_t& spike) {
    if (spike.empty() && !expected) return;
    if (spike.empty() || !expected)
      throw Divergence(std::string(kind) + " differs: rvsim " + (expected ? hex(expected->vaddr) : "none") +
                       ", spike " + (spike.empty() ? "none" : hex(std::get<0>(spike.front()))));
    auto [vaddr, value, bytes] = spike.front();
    bool same_value = std::string(kind) == "load" ||
                      (value & width_mask(bytes)) == (expected->value & width_mask(bytes));
    if (spike.size() != 1 || vaddr != expected->vaddr || bytes != expected->bytes || !same_value)
      throw Divergence(std::string(kind) + " differs: rvsim " + hex(expected->vaddr) + " " +
                       std::to_string(expected->bytes) + " bytes " + hex(expected->value) + ", spike " +
                       hex(vaddr) + " " + std::to_string(bytes) + " bytes " + hex(value));
  }

  bool is_device(uint64_t paddr) { return platform_.addr_to_mem(paddr) == nullptr; }

  insn_t fetch(uint64_t pc) {
    try {
      return hart_.get_mmu()->load_insn(pc).insn;
    } catch (...) {
      return insn_t(0);
    }
  }

  uint64_t read_csr(unsigned addr) {
    try {
      return hart_.get_csr(addr);
    } catch (trap_t&) {
      throw Divergence("rvsim wrote CSR " + hex(addr) + ", which spike does not have");
    }
  }

  bool trapped_to_machine() const { return state_.prv == PRV_M; }
  uint64_t trap_cause() { return read_csr(trapped_to_machine() ? CSR_MCAUSE : CSR_SCAUSE); }
  uint64_t trap_epc() { return read_csr(trapped_to_machine() ? CSR_MEPC : CSR_SEPC); }
  uint64_t trap_tval() { return read_csr(trapped_to_machine() ? CSR_MTVAL : CSR_STVAL); }

  static std::string describe(const std::optional<RegWrite>& write) {
    if (!write) return "no register write";
    return std::string(write->file == RegFile::integer ? "x" : "f") + std::to_string(write->index) + " = " +
           hex(write->value);
  }

  processor_t& hart_;
  state_t& state_;
  Platform& platform_;
  CommitLog& log_;
  uint64_t& commits_;
  bool took_exception_ = false;
  uint64_t instructions_ = 0;
  uint64_t traps_ = 0;
  uint64_t injected_csr_reads_ = 0;
  uint64_t emulated_csr_accesses_ = 0;
  uint64_t failed_store_conditionals_ = 0;
};

ssize_t count_commit_lines(void* cookie, const char* data, size_t size) {
  auto* commits = static_cast<uint64_t*>(cookie);
  for (size_t i = 0; i < size; ++i)
    if (data[i] == '\n') ++*commits;
  return static_cast<ssize_t>(size);
}

// Spike's ISA string for a hart with rvsim's misa: its single-letter
// extensions and those of `extensions` (underscore-separated) whose base
// extension misa has.
std::string isa_string(uint64_t misa, const std::string& extensions) {
  auto has = [misa](char letter) { return (misa >> (letter - 'a')) & 1; };
  std::string isa = "rv64";
  for (char letter : std::string("imafdqcv"))
    if (has(letter)) isa += letter;
  std::istringstream names(extensions);
  std::string name;
  while (std::getline(names, name, '_')) {
    if (name.empty()) continue;
    bool needs_f = name.rfind("zf", 0) == 0 || name.rfind("zvfh", 0) == 0;
    bool needs_v = name.rfind("zv", 0) == 0;
    bool needs_c = name.rfind("zc", 0) == 0;
    if ((needs_f && !has('f')) || (needs_v && !has('v')) || (needs_c && !has('c'))) continue;
    isa += "_" + name;
  }
  return isa;
}

std::string privilege_modes(uint64_t misa) {
  std::string modes = "M";
  if ((misa >> ('s' - 'a')) & 1) modes += "S";
  if ((misa >> ('u' - 'a')) & 1) modes += "U";
  return modes;
}

std::optional<uint64_t> logged_misa(const ResetState& reset) {
  for (auto [addr, value] : reset.csrs)
    if (addr == CSR_MISA) return value;
  return std::nullopt;
}

struct Options {
  std::string extensions;
  uint64_t ram_base = 0x80000000;
  uint64_t ram_size = 0x10000000;
  reg_t pmp_regions = 16;
  reg_t triggers = 2;
  unsigned vaddr_bits = 57;
  std::vector<std::string> elfs;
  std::vector<std::string> raw_loads;
  std::string log;
};

[[noreturn]] void usage() {
  std::cerr << "usage: spike_lockstep --log COMMIT_LOG [--extensions Z1_Z2...] [--elf FILE]...\n"
               "                      [--load FILE@ADDR]... [--ram BASE:SIZE] [--pmp N] [--triggers N]\n"
               "                      [--mmu bare|sv39|sv48|sv57]\n";
  std::exit(2);
}

Options parse_options(int argc, char** argv) {
  Options options;
  for (int i = 1; i < argc; ++i) {
    std::string arg = argv[i];
    auto value = [&]() -> std::string {
      if (i + 1 >= argc) usage();
      return argv[++i];
    };
    if (arg == "--extensions") options.extensions = value();
    else if (arg == "--log") options.log = value();
    else if (arg == "--elf") options.elfs.push_back(value());
    else if (arg == "--load") options.raw_loads.push_back(value());
    else if (arg == "--pmp") options.pmp_regions = std::stoull(value());
    else if (arg == "--triggers") options.triggers = std::stoull(value());
    else if (arg == "--mmu") {
      std::string mode = value();
      if (mode == "bare") options.vaddr_bits = 0;
      else if (mode == "sv39") options.vaddr_bits = 39;
      else if (mode == "sv48") options.vaddr_bits = 48;
      else if (mode == "sv57") options.vaddr_bits = 57;
      else usage();
    }
    else if (arg == "--ram") {
      std::string spec = value();
      auto colon = spec.find(':');
      if (colon == std::string::npos) usage();
      options.ram_base = parse_hex(spec.substr(0, colon));
      options.ram_size = parse_hex(spec.substr(colon + 1));
    } else usage();
  }
  if (options.log.empty()) usage();
  return options;
}

}  // namespace

int main(int argc, char** argv) {
  Options options = parse_options(argc, argv);

  CommitLog log(options.log);
  ResetState reset = log.read_reset();
  std::optional<uint64_t> misa = logged_misa(reset);
  if (!misa) {
    std::cerr << "spike_lockstep: the commit log does not start with rvsim's reset state\n";
    return 2;
  }
  std::string isa = isa_string(*misa, options.extensions);
  std::string priv = privilege_modes(*misa);

  cfg_t cfg;
  cfg.isa = isa.c_str();
  cfg.priv = priv.c_str();
  cfg.pmpregions = options.pmp_regions;
  cfg.trigger_count = options.triggers;
  cfg.hartids = {0};

  Platform platform(cfg, options.ram_base, options.ram_size);
  uint64_t commits = 0;
  cookie_io_functions_t counter{nullptr, count_commit_lines, nullptr, nullptr};
  FILE* commit_sink = fopencookie(&commits, "w", counter);
  setvbuf(commit_sink, nullptr, _IONBF, 0);

  processor_t hart(cfg.isa, cfg.priv, &cfg, &platform, 0, false, commit_sink, std::cerr);
  platform.add_hart(&hart);
  hart.set_max_vaddr_bits(options.vaddr_bits);
  hart.reset();
  hart.enable_log_commits();

  Lockstep lockstep(hart, platform, log, commits);
  try {
    for (const auto& elf : options.elfs) load_elf(platform, elf);
    for (const auto& raw : options.raw_loads) load_raw(platform, raw);
    lockstep.apply_reset(reset);
    lockstep.run();
  } catch (const Divergence& divergence) {
    std::cout << "DIVERGED after " << lockstep.instructions() << " instructions: " << divergence.what() << "\n";
    std::cout << "rvsim log, last lines read:\n";
    for (const auto& line : log.recent()) std::cout << "  " << line << "\n";
    insn_t insn = hart.get_mmu()->load_insn(hart.get_state()->pc).insn;
    std::cout << "spike at " << hex(hart.get_state()->pc) << ": "
              << hart.get_disassembler()->disassemble(insn) << "\n";
    return 1;
  } catch (const std::exception& error) {
    std::cerr << "spike_lockstep: " << error.what() << "\n";
    return 2;
  }
  std::cout << "MATCHED " << lockstep.instructions() << " instructions, " << lockstep.traps() << " traps; "
            << lockstep.injected_csr_reads() << " CSR reads, " << lockstep.emulated_csr_accesses()
            << " accesses to CSRs spike lacks, " << platform.device_reads() << " device reads and "
            << lockstep.failed_store_conditionals() << " SC failures taken from rvsim\n";
  return 0;
}
