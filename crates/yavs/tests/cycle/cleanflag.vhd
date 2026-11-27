-- A81: values that are '0' and '1' at some edges and hold metavalues at others, written by every
-- kind of writer a member has (a package procedure's signal `out` and `inout` formal from a process
-- and as a concurrent call, a record `variable out` formal, package function results assigned
-- whole, through `and`, `not`, a slice, a record field and `&`, a deferred constant, a procedure of
-- the architecture, a memory written by index, a variable kept between runs, an alias of an
-- unresolved signal as a resolved one assigned 'Z', a variable slice as a `variable out` actual, a
-- port actual that is a package function's result, a signal the design forces and releases, a
-- resolved signal a member and a process outside the region both drive), read by
-- combinational and clocked members (an asynchronous reset with a variable, a variable read before
-- it is written in both forms, one larger than the dirty bits are kept for, two processes writing
-- parts of one signal, memories read at an edge where an element holds metavalues: with a variable
-- and an integer updated before the read, and with the element giving an index), `is_x` on '0' and '1', a signal
-- assigned a metavalue literal and then '0' and '1' in one run, against the normal mode.
library ieee;
use ieee.std_logic_1164.all;
package cf_pkg is
  subtype v4 is std_ulogic_vector(3 downto 0);
  type rec_t is record a : v4; end record;
  function fx(c : std_ulogic) return v4;
  function frec(c : std_ulogic) return rec_t;
  procedure put_rec(variable r : out rec_t; c : std_ulogic);
  procedure put_s(signal s : out v4; c : std_ulogic);
  procedure put_io(signal s : inout v4; c : std_ulogic);
  procedure put_v(variable v : out v4; c : std_ulogic);
  constant DC : v4;
end package;
package body cf_pkg is
  function fx(c : std_ulogic) return v4 is
  begin
    if c = '1' then return "X1U1"; end if;
    return "0101";
  end function;
  function frec(c : std_ulogic) return rec_t is begin return (a => fx(c)); end function;
  procedure put_rec(variable r : out rec_t; c : std_ulogic) is begin r.a := fx(c); end procedure;
  procedure put_s(signal s : out v4; c : std_ulogic) is begin s <= fx(c); end procedure;
  procedure put_io(signal s : inout v4; c : std_ulogic) is begin s <= fx(c); end procedure;
  procedure put_v(variable v : out v4; c : std_ulogic) is begin v := fx(c); end procedure;
  constant DC : v4 := "W1-0";
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.cf_pkg.all;
entity cleanflag_sub is
  port (a : in v4; y : out v4);
end entity;
architecture rtl of cleanflag_sub is
begin
  y <= a;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cf_pkg.all;
entity cleanflag_dut is
  port (clk : in std_ulogic; n : out integer_vector(0 to 32));
end entity;
architecture rtl of cleanflag_dut is
  signal cnt : unsigned(3 downto 0) := (others => '0');
  signal ph, ph2 : std_ulogic;
  signal s_s, s_cc, s_io, s_rec, s_fx, s_and, s_not, s_sl, s_fld, s_cat, s_dc, s_arch, s_var, q, s_al :
    v4;
  signal u : std_ulogic_vector(3 downto 0);
  alias al : std_logic_vector(3 downto 0) is u;
  type mem_t is array (0 to 4095) of v4;
  signal mem : mem_t;
  signal r : integer_vector(0 to 32);
  signal s_w65 : signed(3 to 67);
  signal s_q85 : signed(97 downto 3) := to_signed(0, 95);
  signal z0 : std_ulogic := '0';
  signal s_hv, s_rv : v4;
  type mem2_t is array (0 to 4095) of std_ulogic_vector(1 downto 0);
  signal mem2 : mem2_t;
  signal s_q3, s_tab, s_isx, s_dd : v4;
  signal s_vs, s_pe, s_f : v4;
  signal s_mx : std_logic_vector(3 downto 0);
  signal rst : std_ulogic;
  signal s_ar, s_cv, s_bv, q2 : v4;
  signal s_two : std_logic_vector(3 downto 0);
  procedure put_arch(signal s : out v4; c : std_ulogic) is begin s <= fx(c); end procedure;
  -- Counts '1' as 1 and '0' as 100: a metavalue read as a bit changes the sum.
  function ones(v : std_ulogic_vector) return natural is
    variable c : natural := 0;
  begin
    for i in v'range loop
      if v(i) = '1' then c := c + 1; elsif v(i) = '0' then c := c + 100; end if;
    end loop;
    return c;
  end function;
begin
  process (clk) begin
    if rising_edge(clk) then cnt <= cnt + 1; end if;
  end process;
  ph <= cnt(1);
  ph2 <= cnt(2);
  process (ph) begin put_s(s_s, ph); end process;
  put_s(s_cc, ph2);
  process (clk) begin
    if rising_edge(clk) then put_io(s_io, ph); end if;
  end process;
  process (ph)
    variable v : rec_t;
  begin
    v := (a => "0000");
    put_rec(v, ph);
    s_rec <= v.a;
  end process;
  s_fx <= fx(ph2);
  s_and <= fx(ph) and "1111";
  s_not <= not fx(ph2);
  s_sl <= fx(ph)(3 downto 0);
  s_fld <= frec(ph2).a;
  s_cat <= fx(ph)(3 downto 2) & fx(ph2)(1 downto 0);
  s_dc <= DC when ph = '1' else "1010";
  process (ph2) begin put_arch(s_arch, ph2); end process;
  -- A clocked variable: 'U' until the first edge, then a value that holds metavalues at times.
  process (clk)
    variable keep : v4;
  begin
    if rising_edge(clk) then
      if cnt(0) = '1' then keep := fx(ph); end if;
      s_var <= keep;
    end if;
  end process;
  -- A memory written by index: elements 'U' until written, some written with metavalues.
  process (clk) begin
    if rising_edge(clk) then mem(to_integer(cnt)) <= s_s; end if;
  end process;
  q <= mem(to_integer(cnt - 1));
  process (ph, ph2) begin
    if ph = '1' then al <= fx(ph2); else al <= "Z0Z1"; end if;
  end process;
  s_al <= u;
  process (ph)
    variable vv : std_ulogic_vector(7 downto 0);
  begin
    vv := (others => '1');
    put_v(vv(5 downto 2), ph);
    s_vs <= vv(5 downto 2);
  end process;
  sub : entity work.cleanflag_sub port map (a => fx(ph2), y => s_pe);
  s_f <= fx(ph) xor "0110";
  process begin
    wait for 103 ns;
    s_f <= force "1X01";
    wait for 60 ns;
    s_f <= release;
    wait;
  end process;
  s_mx <= std_logic_vector(fx(ph2));
  process begin
    s_mx <= "ZZZZ";
    wait for 205 ns;
    s_mx <= "1ZZZ";
    wait;
  end process;
  rst <= '1' when cnt < 2 else '0';
  -- A signal of more than a word written whole from an integer, at a run-time index in a branch
  -- and whole again in another: the index takes it on the planes from a second pass of the
  -- plane-0 lowering, whose slots must not be the first pass's (cyclefuzz seed 235).
  process (clk) begin
    if rising_edge(clk) then
      if z0 = '1' then
        null;
      else
        s_w65 <= to_signed(0, 65);
        if s_q85(97 downto 90) > 11 then
          s_w65(3 + to_integer(to_unsigned(0, 6))) <= '0';
        elsif std_ulogic_vector(to_unsigned(0, 64)) /= std_ulogic_vector(to_unsigned(0, 64)) then
          s_w65 <= to_signed(0, 65);
        end if;
      end if;
    end if;
  end process;
  -- Variables read before they are written: one holding metavalues at times, one a reset branch
  -- gives metavalues to.
  process (clk)
    variable hv : v4 := "0000";
  begin
    if rising_edge(clk) then
      s_hv <= hv;
      hv := fx(ph);
    end if;
  end process;
  process (clk, rst)
    variable rv : v4;
  begin
    if rst = '1' then
      rv := "XXXX";
    elsif rising_edge(clk) then
      s_rv <= rv;
      rv := std_ulogic_vector(cnt);
    end if;
  end process;
  process (clk, rst)
    variable acc : v4;
  begin
    if rst = '1' then
      acc := "0000";
      s_ar <= "0000";
    elsif rising_edge(clk) then
      acc := acc xor fx(ph);
      s_ar <= acc;
    end if;
  end process;
  process (clk)
    variable cv : unsigned(3 downto 0) := "0000";
  begin
    if rising_edge(clk) then
      cv := cv + 1;
      s_cv <= std_ulogic_vector(cv);
    end if;
  end process;
  process (clk)
    variable bv : std_ulogic_vector(4199 downto 0) := (others => '0');
  begin
    if rising_edge(clk) then
      bv := bv(4198 downto 0) & fx(ph)(3);
      s_bv <= bv(4199 downto 4196) xor bv(3 downto 0);
    end if;
  end process;
  process (clk) begin
    if rising_edge(clk) then s_two(1 downto 0) <= fx(ph2)(1 downto 0); end if;
  end process;
  process (clk) begin
    if rising_edge(clk) then s_two(3 downto 2) <= std_logic_vector(cnt(1 downto 0)); end if;
  end process;
  process (clk) begin
    if rising_edge(clk) then q2 <= mem(to_integer(cnt + 3)) xor "0110"; end if;
  end process;
  process (clk)
    variable acc2 : unsigned(3 downto 0) := "0000";
    variable ic : integer := 0;
  begin
    if rising_edge(clk) then
      acc2 := acc2 + 1;
      ic := (ic + 3) mod 16;
      s_q3 <= mem(to_integer(cnt + 5)) xor std_ulogic_vector(acc2 + to_unsigned(ic, 4));
    end if;
  end process;
  process (clk) begin
    if rising_edge(clk) then
      if ph2 = '1' then mem2(to_integer(cnt)) <= "XX"; else mem2(to_integer(cnt)) <= "01"; end if;
    end if;
  end process;
  process (clk)
    variable k : integer;
    constant tab : integer_vector(0 to 2) := (5, 7, 9);
  begin
    if rising_edge(clk) then
      k := to_integer(unsigned(mem2(to_integer(cnt - 1))));
      s_tab <= std_ulogic_vector(to_unsigned(tab(k), 4));
    end if;
  end process;
  s_isx <= "0001" when is_x(s_cv) else "0010";
  process (ph, cnt) begin
    s_dd <= "X0X0";
    if ph = '1' then s_dd <= std_ulogic_vector(cnt); end if;
  end process;
  r <= (ones(s_s), ones(s_cc), ones(s_io), ones(s_rec), ones(s_fx), ones(s_and), ones(s_not),
    ones(s_sl), ones(s_fld), ones(s_cat), ones(s_dc), ones(s_arch), ones(s_var), ones(q),
    ones(s_al), ones(s_s) + ones(s_cc), ones(s_ar), ones(s_cv), ones(s_bv), ones(s_two), ones(q2),
    ones(s_ar) + ones(s_two), ones(s_vs), ones(s_pe), ones(s_f), ones(s_mx), ones(s_q3), ones(s_tab),
    ones(s_isx), ones(s_dd), ones(s_hv), ones(s_rv), ones(std_ulogic_vector(s_w65)));
  process (clk) begin
    if rising_edge(clk) then n <= r; end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
entity cycle_cleanflag is end entity;
architecture tb of cycle_cleanflag is
  signal clk : std_ulogic := '0';
  signal n : integer_vector(0 to 32);
begin
  dut : entity work.cleanflag_dut port map (clk, n);
  process
    variable t : integer;
  begin
    for i in 1 to 40 loop
      clk <= '1'; wait for 5 ns; clk <= '0'; wait for 5 ns;
      t := 0;
      for k in n'range loop t := (t * 7 + n(k)) mod 1000003; end loop;
      report "edge=" & integer'image(i) & " sum=" & integer'image(t);
    end loop;
    wait;
  end process;
end architecture;
