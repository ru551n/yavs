-- pgo: top=rec_pipe_tb std=2008 stop=2400 us
-- Reproduces a small in-order four stage core written in the two-process style:
-- record registers r/rin, whole records on stage ports, a register file array,
-- an ALU with numeric_std arithmetic, hazards, branch flushes and a model check.

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package rp_pkg is
  subtype word_t is std_logic_vector(31 downto 0);
  subtype dword_t is std_logic_vector(63 downto 0);
  subtype reg_idx_t is integer range 0 to 31;
  subtype pc_t is unsigned(15 downto 0);
  subtype imm_t is signed(31 downto 0);
  constant PROG_LEN : natural := 64;

  type op_t is (OP_NOP, OP_ADD, OP_SUB, OP_ADC, OP_AND, OP_OR, OP_XOR, OP_SLL, OP_SRL,
                OP_ROL, OP_SLT, OP_MUL, OP_ADDI, OP_LUI, OP_ORI, OP_SLLI, OP_SRLI,
                OP_ANDI, OP_BEQ, OP_BNE, OP_BLT, OP_JMP, OP_HALT, OP_BAD);
  type class_t is (C_NONE, C_ALU, C_IMM, C_BR, C_JMP, C_HALT);

  type regs_t is array (reg_idx_t) of dword_t;
  type prog_t is array (0 to PROG_LEN - 1) of word_t;

  type flags_t is record
    z, n, c, v : std_ulogic;
  end record;
  constant FLAGS_0 : flags_t := (others => '0');

  type ctl_t is record
    cls : class_t;
    wr  : std_ulogic;
  end record;
  constant CTL_0 : ctl_t := (cls => C_NONE, wr => '0');

  type fetch_t is record
    valid : std_ulogic;
    pc    : pc_t;
    instr : word_t;
  end record;
  constant FETCH_IDLE : fetch_t := ('0', (others => '0'), (others => '0'));

  type dec_t is record
    valid  : std_ulogic;
    pc     : pc_t;
    op     : op_t;
    ctl    : ctl_t;
    rd     : reg_idx_t;
    a, b   : dword_t;
    target : pc_t;
  end record;
  constant DEC_IDLE : dec_t := ('0', (others => '0'), OP_NOP, CTL_0, 0,
                                (others => '0'), (others => '0'), (others => '0'));

  type exe_t is record
    valid    : std_ulogic;
    rd       : reg_idx_t;
    wr       : std_ulogic;
    data     : dword_t;
    redirect : std_ulogic;
    target   : pc_t;
    halt     : std_ulogic;
    fl       : flags_t;
  end record;
  constant EXE_IDLE : exe_t := ('0', 0, '0', (others => '0'), '0', (others => '0'), '0', FLAGS_0);

  type wb_t is record
    en   : std_ulogic;
    idx  : reg_idx_t;
    data : dword_t;
  end record;
  constant WB_IDLE : wb_t := ('0', 0, (others => '0'));

  type retire_t is record
    count  : unsigned(31 downto 0);
    halted : std_ulogic;
    fl     : flags_t;
  end record;

  type hz_t is record
    valid : std_ulogic;
    rd    : reg_idx_t;
  end record;
  type hz_arr_t is array (natural range <>) of hz_t;

  type rd_req_t is record
    ra, rb : reg_idx_t;
  end record;
  type rd_rsp_t is record
    a, b : dword_t;
  end record;

  type model_t is record
    regs  : regs_t;
    count : natural;
    fl    : flags_t;
  end record;

  function to_sl(b : boolean) return std_ulogic;
  function class_of(op : op_t) return class_t;
  function decode_op(code : std_logic_vector(5 downto 0)) return op_t;
  function enc_r(op : op_t; rd, rs1, rs2 : natural) return word_t;
  function enc_i(op : op_t; rd, rs1 : natural; imm : integer) return word_t;
  function imm_for(op : op_t; instr : word_t) return imm_t;
  function sets_flags(op : op_t) return boolean;
  function flags_of(res : dword_t; c, v : std_ulogic) return flags_t;
  function branch_taken(op : op_t; a, b : dword_t) return boolean;
  function low_word(x : dword_t) return unsigned;
  procedure alu(op : in op_t; a, b : in dword_t; cin : in std_ulogic;
                res : out dword_t; c_out, v_out : out std_ulogic);
  procedure expect(errs : inout natural; name : in string; got, want : in std_logic_vector);
  function prog_checksum(n : natural) return prog_t;
  function prog_fib(n : natural) return prog_t;
  function model_run(prog : prog_t; max_steps : natural) return model_t;
end package;

package body rp_pkg is
  function to_sl(b : boolean) return std_ulogic is
  begin
    if b then
      return '1';
    end if;
    return '0';
  end function;

  function class_of(op : op_t) return class_t is
  begin
    case op is
      when OP_ADD to OP_MUL   => return C_ALU;
      when OP_ADDI to OP_ANDI => return C_IMM;
      when OP_BEQ to OP_BLT   => return C_BR;
      when OP_JMP             => return C_JMP;
      when OP_HALT            => return C_HALT;
      when others             => return C_NONE;
    end case;
  end function;

  function decode_op(code : std_logic_vector(5 downto 0)) return op_t is
    variable n : natural := to_integer(unsigned(code));
  begin
    if n < op_t'pos(OP_BAD) then
      return op_t'val(n);
    end if;
    return OP_BAD;
  end function;

  function enc_r(op : op_t; rd, rs1, rs2 : natural) return word_t is
  begin
    return std_logic_vector(to_unsigned(op_t'pos(op), 6))
         & std_logic_vector(to_unsigned(rd, 5))
         & std_logic_vector(to_unsigned(rs1, 5))
         & std_logic_vector(to_unsigned(rs2, 5))
         & "00000000000";
  end function;

  function enc_i(op : op_t; rd, rs1 : natural; imm : integer) return word_t is
    variable low : std_logic_vector(15 downto 0);
  begin
    if imm < 0 then
      low := std_logic_vector(to_signed(imm, 16));
    else
      low := std_logic_vector(to_unsigned(imm, 16));
    end if;
    return std_logic_vector(to_unsigned(op_t'pos(op), 6))
         & std_logic_vector(to_unsigned(rd, 5))
         & std_logic_vector(to_unsigned(rs1, 5))
         & low;
  end function;

  function imm_for(op : op_t; instr : word_t) return imm_t is
    alias imm16 : std_logic_vector(15 downto 0) is instr(15 downto 0);
  begin
    case op is
      when OP_ADDI =>
        return resize(signed(imm16), 32);
      when OP_ORI | OP_ANDI | OP_SLLI | OP_SRLI =>
        return signed(resize(unsigned(imm16), 32));
      when OP_LUI =>
        return signed(std_logic_vector'(imm16 & x"0000"));
      when others =>
        return (others => '0');
    end case;
  end function;

  function sets_flags(op : op_t) return boolean is
  begin
    return op = OP_ADD or op = OP_SUB or op = OP_ADC or op = OP_ADDI;
  end function;

  function flags_of(res : dword_t; c, v : std_ulogic) return flags_t is
    variable f : flags_t;
  begin
    f.z := to_sl(res = dword_t'(others => '0'));
    f.n := res(res'left);
    f.c := c;
    f.v := v;
    return f;
  end function;

  function branch_taken(op : op_t; a, b : dword_t) return boolean is
  begin
    case op is
      when OP_BEQ => return a = b;
      when OP_BNE => return a /= b;
      when OP_BLT => return signed(a) < signed(b);
      when others => return false;
    end case;
  end function;

  function low_word(x : dword_t) return unsigned is
  begin
    return unsigned(x(31 downto 0));
  end function;

  procedure alu(op : in op_t; a, b : in dword_t; cin : in std_ulogic;
                res : out dword_t; c_out, v_out : out std_ulogic) is
    variable ua   : unsigned(63 downto 0) := unsigned(a);
    variable ub   : unsigned(63 downto 0) := unsigned(b);
    variable wide : unsigned(64 downto 0);
    variable r    : unsigned(63 downto 0) := (others => '0');
    variable amt  : natural range 0 to 63;
    variable c, v : std_ulogic := '0';
    variable cy   : unsigned(0 downto 0);
  begin
    amt := to_integer(ub(5 downto 0));
    case op is
      when OP_ADD | OP_ADDI =>
        wide := ('0' & ua) + ('0' & ub);
        r := wide(63 downto 0);
        c := wide(64);
        v := (not (a(63) xor b(63))) and (a(63) xor r(63));
      when OP_ADC =>
        cy(0) := cin;
        wide := ('0' & ua) + ('0' & ub) + resize(cy, 65);
        r := wide(63 downto 0);
        c := wide(64);
        v := (not (a(63) xor b(63))) and (a(63) xor r(63));
      when OP_SUB =>
        wide := ('0' & ua) - ('0' & ub);
        r := wide(63 downto 0);
        c := wide(64);
        v := (a(63) xor b(63)) and (a(63) xor r(63));
      when OP_AND | OP_ANDI => r := ua and ub;
      when OP_OR | OP_ORI   => r := ua or ub;
      when OP_XOR           => r := ua xor ub;
      when OP_SLL | OP_SLLI => r := shift_left(ua, amt);
      when OP_SRL | OP_SRLI => r := shift_right(ua, amt);
      when OP_ROL           => r := rotate_left(ua, amt);
      when OP_SLT =>
        if signed(a) < signed(b) then
          r(0) := '1';
        end if;
      when OP_MUL => r := resize(ua(31 downto 0) * ub(31 downto 0), 64);
      when OP_LUI => r := ub;
      when others => null;
    end case;
    res := std_logic_vector(r);
    c_out := c;
    v_out := v;
  end procedure;

  procedure expect(errs : inout natural; name : in string; got, want : in std_logic_vector) is
  begin
    if got /= want then
      errs := errs + 1;
      report "mismatch in " & name & ": got " & to_hstring(got) & " want " & to_hstring(want)
        severity error;
    end if;
  end procedure;

  -- Checksum loop: multiply, shifts, rotate, carry chain, compares and all branch kinds.
  function prog_checksum(n : natural) return prog_t is
    variable p : prog_t := (others => (others => '0'));
  begin
    p(0)  := enc_r(OP_NOP, 0, 0, 0);
    p(1)  := enc_i(OP_ADDI, 1, 0, n);
    p(2)  := enc_i(OP_ADDI, 2, 0, 16#1357#);
    p(3)  := enc_i(OP_LUI, 3, 0, 16#2468#);
    p(4)  := enc_i(OP_ORI, 3, 3, 16#ACE1#);
    p(5)  := enc_i(OP_ADDI, 4, 0, 13);
    p(6)  := enc_i(OP_ADDI, 7, 0, 5);
    p(7)  := enc_i(OP_ADDI, 11, 0, 29);
    -- loop body starts at 8
    p(8)  := enc_r(OP_MUL, 5, 3, 3);
    p(9)  := enc_r(OP_ADD, 2, 2, 5);
    p(10) := enc_r(OP_XOR, 6, 2, 3);
    p(11) := enc_r(OP_ROL, 6, 6, 4);
    p(12) := enc_r(OP_SLL, 8, 2, 7);
    p(13) := enc_r(OP_SRL, 10, 2, 11);
    p(14) := enc_r(OP_ADC, 2, 6, 10);
    p(15) := enc_r(OP_SUB, 9, 8, 10);
    p(16) := enc_r(OP_AND, 5, 9, 3);
    p(17) := enc_r(OP_OR, 12, 12, 5);
    p(18) := enc_r(OP_SLT, 13, 9, 2);
    p(19) := enc_r(OP_ADD, 12, 12, 13);
    p(20) := enc_i(OP_BEQ, 13, 0, 3);
    p(21) := enc_r(OP_XOR, 2, 2, 9);
    p(22) := enc_i(OP_BLT, 9, 2, 2);
    p(23) := enc_i(OP_ADDI, 14, 14, 1);
    p(24) := enc_i(OP_ADDI, 3, 3, 16#1F3D#);
    p(25) := enc_i(OP_SLLI, 5, 3, 7);
    p(26) := enc_i(OP_SRLI, 6, 3, 3);
    p(27) := enc_r(OP_XOR, 3, 3, 5);
    p(28) := enc_r(OP_XOR, 3, 3, 6);
    p(29) := enc_i(OP_ADDI, 1, 1, -1);
    p(30) := enc_i(OP_BNE, 1, 0, 2);
    p(31) := enc_i(OP_HALT, 0, 0, 0);
    p(32) := enc_i(OP_JMP, 0, 0, -24);
    return p;
  end function;

  -- Fibonacci-like recurrence with shifts.
  function prog_fib(n : natural) return prog_t is
    variable p : prog_t := (others => (others => '0'));
  begin
    p(0)  := enc_i(OP_ADDI, 1, 0, n);
    p(1)  := enc_i(OP_ADDI, 2, 0, 1);
    p(2)  := enc_i(OP_ADDI, 3, 0, 1);
    p(3)  := enc_i(OP_ADDI, 4, 0, 3);
    p(4)  := enc_r(OP_ADD, 5, 2, 3);
    p(5)  := enc_r(OP_SLL, 6, 5, 4);
    p(6)  := enc_r(OP_XOR, 2, 3, 6);
    p(7)  := enc_r(OP_ADD, 3, 5, 0);
    p(8)  := enc_i(OP_SRLI, 7, 3, 2);
    p(9)  := enc_r(OP_SUB, 8, 8, 7);
    p(10) := enc_i(OP_ADDI, 1, 1, -1);
    p(11) := enc_i(OP_BNE, 1, 0, -7);
    p(12) := enc_i(OP_HALT, 0, 0, 0);
    return p;
  end function;

  -- Reference interpreter, one instruction at a time, no pipeline.
  function model_run(prog : prog_t; max_steps : natural) return model_t is
    variable m     : model_t;
    variable pc    : integer := 0;
    variable steps : natural := 0;
    variable instr : word_t;
    variable op    : op_t;
    variable cls   : class_t;
    variable rd    : natural;
    variable rs1   : natural;
    variable rs2   : natural;
    variable a, b  : dword_t;
    variable res   : dword_t;
    variable c, v  : std_ulogic;
    variable nxt   : integer;
  begin
    m.regs := (others => (others => '0'));
    m.count := 0;
    m.fl := FLAGS_0;
    loop
      assert steps < max_steps report "model ran away" severity failure;
      steps := steps + 1;
      instr := prog(pc mod PROG_LEN);
      op := decode_op(instr(31 downto 26));
      cls := class_of(op);
      rd := to_integer(unsigned(instr(25 downto 21)));
      rs1 := to_integer(unsigned(instr(20 downto 16)));
      rs2 := to_integer(unsigned(instr(15 downto 11)));
      nxt := pc + 1;
      m.count := m.count + 1;
      case cls is
        when C_ALU | C_IMM =>
          a := m.regs(rs1);
          if cls = C_ALU then
            b := m.regs(rs2);
          else
            b := std_logic_vector(resize(imm_for(op, instr), 64));
          end if;
          alu(op, a, b, m.fl.c, res, c, v);
          if rd /= 0 then
            m.regs(rd) := res;
          end if;
          if sets_flags(op) then
            m.fl := flags_of(res, c, v);
          end if;
        when C_BR =>
          if branch_taken(op, m.regs(rd), m.regs(rs1)) then
            nxt := pc + to_integer(signed(instr(15 downto 0)));
          end if;
        when C_JMP =>
          nxt := pc + to_integer(signed(instr(15 downto 0)));
        when C_HALT =>
          exit;
        when C_NONE =>
          assert op /= OP_BAD report "bad opcode at " & integer'image(pc) severity failure;
      end case;
      pc := nxt;
    end loop;
    return m;
  end function;
end package body;

----------------------------------------------------------------------------
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.rp_pkg.all;

entity rp_regfile is
  port (
    clk  : in  std_ulogic;
    rq   : in  rd_req_t;
    rsp  : out rd_rsp_t;
    wp   : in  wb_t;
    dump : out regs_t
  );
end entity;

architecture rtl of rp_regfile is
  signal regs : regs_t := (others => (others => '0'));
begin
  rsp <= (a => regs(rq.ra), b => regs(rq.rb));
  dump <= regs;

  wr : process (clk)
  begin
    if rising_edge(clk) then
      if wp.en = '1' and wp.idx /= 0 then
        regs(wp.idx) <= wp.data;
      end if;
    end if;
  end process;
end architecture;

----------------------------------------------------------------------------
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.rp_pkg.all;

entity rp_fetch is
  generic (g_prog : prog_t);
  port (
    clk, rst : in  std_ulogic;
    stall    : in  std_ulogic;
    flush    : in  std_ulogic;
    target   : in  pc_t;
    halted   : in  std_ulogic;
    fout     : out fetch_t := FETCH_IDLE
  );
end entity;

architecture rtl of rp_fetch is
  type fe_state_t is (F_RESET, F_RUN, F_DRAIN);
  type reg_t is record
    st     : fe_state_t;
    pc     : pc_t;
    o      : fetch_t;
    nfetch : unsigned(23 downto 0);
  end record;
  constant R_INIT : reg_t := (F_RESET, (others => '0'), FETCH_IDLE, (others => '0'));
  signal r, rin : reg_t := R_INIT;
begin
  comb : process (all)
    variable v : reg_t;
  begin
    v := r;
    case r.st is
      when F_RESET =>
        v.pc := (others => '0');
        v.o := FETCH_IDLE;
        v.st := F_RUN;
      when F_RUN =>
        if flush = '1' then
          v.pc := target;
          v.o := FETCH_IDLE;
        elsif stall = '0' then
          v.o := ('1', r.pc, g_prog(to_integer(r.pc(5 downto 0))));
          v.pc := r.pc + 1;
          v.nfetch := r.nfetch + 1;
        end if;
        if halted = '1' then
          v.st := F_DRAIN;
        end if;
      when F_DRAIN =>
        v.o := FETCH_IDLE;
    end case;
    if rst = '1' then
      v := R_INIT;
    end if;
    rin <= v;
  end process;

  seq : process (clk)
  begin
    if rising_edge(clk) then
      r <= rin;
    end if;
  end process;

  fout <= r.o;
end architecture;

----------------------------------------------------------------------------
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.rp_pkg.all;

entity rp_decode is
  port (
    clk, rst : in  std_ulogic;
    fin      : in  fetch_t;
    flush    : in  std_ulogic;
    busy     : in  std_ulogic;
    hz       : in  hz_arr_t(0 to 1);
    rq       : out rd_req_t;
    rsp      : in  rd_rsp_t;
    stall    : out std_ulogic;
    dout     : out dec_t
  );
end entity;

architecture rtl of rp_decode is
  signal r, rin : dec_t := DEC_IDLE;
  alias f_op  : std_logic_vector(5 downto 0) is fin.instr(31 downto 26);
  alias f_rd  : std_logic_vector(4 downto 0) is fin.instr(25 downto 21);
  alias f_rs1 : std_logic_vector(4 downto 0) is fin.instr(20 downto 16);
  alias f_rs2 : std_logic_vector(4 downto 0) is fin.instr(15 downto 11);
  alias f_imm : std_logic_vector(15 downto 0) is fin.instr(15 downto 0);
begin
  comb : process (all)
    variable v      : dec_t;
    variable op     : op_t;
    variable cls    : class_t;
    variable rd     : reg_idx_t;
    variable ra, rb : reg_idx_t;
    variable haz    : boolean;
    variable stl    : std_ulogic;
  begin
    op := decode_op(f_op);
    cls := class_of(op);
    rd := to_integer(unsigned(f_rd));
    case cls is
      when C_ALU =>
        ra := to_integer(unsigned(f_rs1));
        rb := to_integer(unsigned(f_rs2));
      when C_IMM =>
        ra := to_integer(unsigned(f_rs1));
        rb := 0;
      when C_BR =>
        ra := rd;
        rb := to_integer(unsigned(f_rs1));
      when others =>
        ra := 0;
        rb := 0;
    end case;
    rq <= (ra => ra, rb => rb);

    haz := false;
    for i in hz'range loop
      if hz(i).valid = '1' and hz(i).rd /= 0 and (hz(i).rd = ra or hz(i).rd = rb) then
        haz := true;
      end if;
    end loop;
    if r.valid = '1' and r.ctl.wr = '1' and (r.rd = ra or r.rd = rb) then
      haz := true;
    end if;

    stl := busy or (fin.valid and to_sl(haz) and not flush);
    v := r;
    if flush = '1' then
      v.valid := '0';
    elsif busy = '1' then
      null;
    elsif fin.valid = '1' and haz then
      v.valid := '0';
    elsif fin.valid = '1' then
      v.valid := '1';
      v.pc := fin.pc;
      v.op := op;
      v.rd := rd;
      v.ctl.cls := cls;
      v.ctl.wr := to_sl((cls = C_ALU or cls = C_IMM) and rd /= 0);
      v.a := rsp.a;
      if cls = C_IMM then
        v.b := std_logic_vector(resize(imm_for(op, fin.instr), 64));
      else
        v.b := rsp.b;
      end if;
      v.target := fin.pc + unsigned(resize(signed(f_imm), 16));
    else
      v.valid := '0';
    end if;
    if rst = '1' then
      v := DEC_IDLE;
    end if;
    rin <= v;
    stall <= stl;
  end process;

  seq : process (clk)
  begin
    if rising_edge(clk) then
      r <= rin;
    end if;
  end process;

  dout <= r;
end architecture;

----------------------------------------------------------------------------
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.rp_pkg.all;

entity rp_execute is
  port (
    clk, rst : in  std_ulogic;
    din      : in  dec_t;
    flush    : in  std_ulogic;
    busy     : out std_ulogic;
    eout     : out exe_t
  );
end entity;

architecture rtl of rp_execute is
  type ex_state_t is (E_IDLE, E_MULW);
  type reg_t is record
    st     : ex_state_t;
    o      : exe_t;
    fl     : flags_t;
    prod   : unsigned(63 downto 0);
    halted : std_ulogic;
  end record;
  constant R_INIT : reg_t := (E_IDLE, EXE_IDLE, FLAGS_0, (others => '0'), '0');
  signal r, rin : reg_t := R_INIT;
begin
  comb : process (all)
    variable v    : reg_t;
    variable res  : dword_t;
    variable c, x : std_ulogic;
  begin
    v := r;
    v.o.valid := '0';
    v.o.redirect := '0';
    v.o.wr := '0';
    v.o.halt := '0';
    busy <= '0';
    case r.st is
      when E_IDLE =>
        if din.valid = '1' and flush = '0' and r.halted = '0' then
          case din.ctl.cls is
            when C_ALU | C_IMM =>
              if din.op = OP_MUL then
                v.st := E_MULW;
                v.prod := low_word(din.a) * low_word(din.b);
                busy <= '1';
              else
                alu(din.op, din.a, din.b, r.fl.c, res, c, x);
                v.o.valid := '1';
                v.o.rd := din.rd;
                v.o.wr := din.ctl.wr;
                v.o.data := res;
                if sets_flags(din.op) then
                  v.fl := flags_of(res, c, x);
                end if;
              end if;
            when C_BR =>
              v.o.valid := '1';
              v.o.redirect := to_sl(branch_taken(din.op, din.a, din.b));
              v.o.target := din.target;
            when C_JMP =>
              v.o.valid := '1';
              v.o.redirect := '1';
              v.o.target := din.target;
            when C_HALT =>
              v.o.valid := '1';
              v.o.halt := '1';
              v.halted := '1';
            when C_NONE =>
              v.o.valid := '1';
          end case;
        end if;
      when E_MULW =>
        v.o.valid := '1';
        v.o.rd := din.rd;
        v.o.wr := din.ctl.wr;
        v.o.data := std_logic_vector(r.prod);
        v.st := E_IDLE;
    end case;
    v.o.fl := v.fl;
    if rst = '1' then
      v := R_INIT;
    end if;
    rin <= v;
  end process;

  seq : process (clk)
  begin
    if rising_edge(clk) then
      r <= rin;
    end if;
  end process;

  eout <= r.o;
end architecture;

----------------------------------------------------------------------------
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.rp_pkg.all;

entity rp_wb is
  port (
    clk, rst : in  std_ulogic;
    ein      : in  exe_t;
    wport    : out wb_t;
    ret      : out retire_t
  );
end entity;

architecture rtl of rp_wb is
  type reg_t is record
    w   : wb_t;
    ret : retire_t;
  end record;
  constant R_INIT : reg_t := (WB_IDLE, ((others => '0'), '0', FLAGS_0));
  signal r, rin : reg_t := R_INIT;
begin
  comb : process (all)
    variable v : reg_t;
  begin
    v := r;
    v.w.en := '0';
    if ein.valid = '1' then
      v.ret.count := r.ret.count + 1;
      v.ret.fl := ein.fl;
      if ein.wr = '1' then
        v.w := ('1', ein.rd, ein.data);
      end if;
      if ein.halt = '1' then
        v.ret.halted := '1';
      end if;
    end if;
    if rst = '1' then
      v := R_INIT;
    end if;
    rin <= v;
  end process;

  seq : process (clk)
  begin
    if rising_edge(clk) then
      r <= rin;
    end if;
  end process;

  wport <= r.w;
  ret <= r.ret;
end architecture;

----------------------------------------------------------------------------
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.rp_pkg.all;

entity rp_core is
  generic (
    g_prog : prog_t;
    g_name : string := "core"
  );
  port (
    clk, rst : in  std_ulogic;
    ret      : out retire_t;
    dump     : out regs_t
  );
end entity;

architecture rtl of rp_core is
  signal fo     : fetch_t := FETCH_IDLE;
  signal dout   : dec_t;
  signal eo     : exe_t;
  signal wp     : wb_t;
  signal rq     : rd_req_t;
  signal rsp    : rd_rsp_t;
  signal hz     : hz_arr_t(0 to 1);
  signal stall  : std_ulogic;
  signal busy   : std_ulogic;
  signal flush  : std_ulogic;
  signal retire : retire_t;
begin
  assert g_prog'length = PROG_LEN
    report g_name & ": program has " & integer'image(g_prog'length) & " words"
    severity failure;

  flush <= eo.valid and eo.redirect;
  hz <= (0 => (valid => eo.valid and eo.wr, rd => eo.rd),
         1 => (valid => wp.en, rd => wp.idx));

  u_fetch : entity work.rp_fetch
    generic map (g_prog => g_prog)
    port map (clk => clk, rst => rst, stall => stall, flush => flush,
              target => eo.target, halted => retire.halted, fout => fo);

  u_decode : entity work.rp_decode
    port map (clk => clk, rst => rst, fin => fo, flush => flush, busy => busy,
              hz => hz, rq => rq, rsp => rsp, stall => stall, dout => dout);

  u_exec : entity work.rp_execute
    port map (clk => clk, rst => rst, din => dout, flush => flush, busy => busy, eout => eo);

  u_wb : entity work.rp_wb
    port map (clk => clk, rst => rst, ein => eo, wport => wp, ret => retire);

  u_regs : entity work.rp_regfile
    port map (clk => clk, rq => rq, rsp => rsp, wp => wp, dump => dump);

  ret <= retire;
end architecture;

----------------------------------------------------------------------------
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.rp_pkg.all;

entity rec_pipe_tb is
end entity;

architecture sim of rec_pipe_tb is
  constant NCORES : natural := 2;
  type progs_t is array (0 to NCORES - 1) of prog_t;
  type rets_t is array (0 to NCORES - 1) of retire_t;
  type dumps_t is array (0 to NCORES - 1) of regs_t;
  constant PROGS : progs_t := (0 => prog_checksum(2800), 1 => prog_fib(2100));

  signal clk   : std_ulogic := '0';
  signal rst   : std_ulogic := '1';
  signal done  : boolean := false;
  signal rets  : rets_t;
  signal dumps : dumps_t;
begin
  clk <= not clk after 5 ns when not done else '0';
  rst <= '0' after 42 ns;

  cores : for i in 0 to NCORES - 1 generate
    u_core : entity work.rp_core
      generic map (g_prog => PROGS(i), g_name => "core" & integer'image(i))
      port map (clk => clk, rst => rst, ret => rets(i), dump => dumps(i));
  end generate;

  -- Register zero must stay zero at all times.
  mon : process (clk)
  begin
    if rising_edge(clk) then
      for i in 0 to NCORES - 1 loop
        assert dumps(i)(0) = dword_t'(others => '0') report "r0 changed" severity failure;
      end loop;
    end if;
  end process;

  chk : process
    variable m    : model_t;
    variable errs : natural := 0;
    variable cyc  : natural := 0;
  begin
    wait until rst = '0';
    while not (rets(0).halted = '1' and rets(1).halted = '1') loop
      wait until rising_edge(clk);
      cyc := cyc + 1;
      assert cyc < 400000 report "cores did not halt" severity failure;
    end loop;
    for k in 1 to 6 loop
      wait until rising_edge(clk);
    end loop;
    for i in 0 to NCORES - 1 loop
      m := model_run(PROGS(i), 1000000);
      for k in regs_t'range loop
        expect(errs, "core" & integer'image(i) & " r" & integer'image(k), dumps(i)(k), m.regs(k));
      end loop;
      expect(errs, "retired", std_logic_vector(rets(i).count),
             std_logic_vector(to_unsigned(m.count, 32)));
      expect(errs, "flags", rets(i).fl.z & rets(i).fl.n & rets(i).fl.c & rets(i).fl.v,
             m.fl.z & m.fl.n & m.fl.c & m.fl.v);
    end loop;
    assert errs = 0 report integer'image(errs) & " mismatches" severity failure;
    report "cycles " & integer'image(cyc) severity note;
    report "PASS" severity note;
    done <= true;
    wait;
  end process;
end architecture;
