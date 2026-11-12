-- difftest: top=memo_pure_calls std=2008
-- Pure functions called from a loop with arguments that repeat: cheap ones without a loop,
-- recursion, concatenation of bytes or native call but `ieee` gates (compiled callers call
-- them without the memo), one that calls another, one without a loop that calls itself, one with
-- a loop that reports (each call must report), and functions whose results do not
-- follow from their arguments (impure: a signal, a shared variable, a file, a process variable;
-- a function nested in a process): these must run every time. Each result is checked against
-- the same value computed inline.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
package memo_pure_calls_pkg is
  type count_t is protected
    procedure bump;
    impure function get return integer;
  end protected;
  function ones(v : std_ulogic_vector) return natural;
  function pick(op : std_ulogic_vector(2 downto 0); a, b : std_ulogic_vector(7 downto 0)) return std_ulogic_vector;
  function pick2(op : std_ulogic_vector(2 downto 0); a, b : std_ulogic_vector(7 downto 0)) return std_ulogic_vector;
  function noisy(v : std_ulogic_vector) return natural;
  function parity(v : std_ulogic_vector) return std_ulogic;
  impure function rd(file ff : std.textio.text) return integer;
end package;
package body memo_pure_calls_pkg is
  type count_t is protected body
    variable n : integer := 0;
    procedure bump is begin n := n + 1; end;
    impure function get return integer is begin return n; end;
  end protected body;
  function ones(v : std_ulogic_vector) return natural is
    variable n : natural := 0;
  begin
    for i in v'range loop
      if v(i) = '1' then n := n + 1; end if;
    end loop;
    return n;
  end;
  function pick(op : std_ulogic_vector(2 downto 0); a, b : std_ulogic_vector(7 downto 0)) return std_ulogic_vector is
    variable r : std_ulogic_vector(7 downto 0);
  begin
    case op is
      when "000" => r := a and b;
      when "001" => r := a or b;
      when "010" => r := a xor b;
      when "011" => r := not a;
      when "100" => r := a nor b;
      when "101" => r := (a nand b) xor b;
      when "110" => r := a xnor b;
      when others => r := b;
    end case;
    return r;
  end;
  function pick2(op : std_ulogic_vector(2 downto 0); a, b : std_ulogic_vector(7 downto 0)) return std_ulogic_vector is
  begin
    return pick(op, b, a) xor pick(op, a, b);
  end;
  function parity(v : std_ulogic_vector) return std_ulogic is
    alias w : std_ulogic_vector(v'length - 1 downto 0) is v;
  begin
    if w'length = 1 then
      return w(0);
    end if;
    return parity(w(w'left downto w'length / 2)) xor parity(w(w'length / 2 - 1 downto 0));
  end;
  function noisy(v : std_ulogic_vector) return natural is
    variable n : natural := 0;
  begin
    for i in v'range loop
      if v(i) = '1' then n := n + 1; end if;
    end loop;
    report "noisy " & integer'image(n);
    return n;
  end;
  impure function rd(file ff : std.textio.text) return integer is
    variable l : std.textio.line;
    variable v : integer;
  begin
    std.textio.readline(ff, l);
    std.textio.read(l, v);
    return v;
  end;
end package body;

library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all; use std.textio.all;
use work.memo_pure_calls_pkg.all;
entity memo_pure_calls is end;
architecture a of memo_pure_calls is
  signal s : natural := 0;
  shared variable cnt : count_t;
  impure function from_sig(x : natural) return natural is begin return x + s; end;
  impure function from_shared(x : natural) return natural is begin cnt.bump; return x + cnt.get; end;
begin
  process
    variable a, b : std_ulogic_vector(7 downto 0);
    variable op : std_ulogic_vector(2 downto 0);
    variable r, e : std_ulogic_vector(7 downto 0);
    variable sum_ones, sum_pick, sum_pick2, sum_par, sum_sig, sum_sh, sum_var, sum_nest : integer := 0;
    variable k : natural := 0;
    variable acc : natural := 0;
    impure function from_var(x : natural) return natural is begin acc := acc + 1; return x + acc; end;
    function nested(x : natural) return natural is
      variable n : natural := 0;
    begin
      for i in 0 to x mod 5 loop n := n + i; end loop;
      return n;
    end;
    file f : text;
    variable l : line;
    variable fv : integer;
    variable sum_file : integer := 0;
  begin
    file_open(f, "memo_pure_calls.txt", write_mode);
    for i in 0 to 5 loop write(l, i * 3); writeline(f, l); end loop;
    file_close(f);
    for i in 0 to 2999 loop
      k := i mod 7;
      a := std_ulogic_vector(to_unsigned(k * 37 mod 256, 8));
      b := std_ulogic_vector(to_unsigned((k * 11 + 3) mod 256, 8));
      op := std_ulogic_vector(to_unsigned(k mod 8, 3));
      -- With a loop.
      e := a;
      assert ones(a) = ones(e) severity failure;
      sum_ones := sum_ones + ones(a & b);
      -- Without a loop, and one calling it.
      r := pick(op, a, b);
      case to_integer(unsigned(op)) is
        when 0 => e := a and b;
        when 1 => e := a or b;
        when 2 => e := a xor b;
        when 3 => e := not a;
        when 4 => e := a nor b;
        when 5 => e := (a nand b) xor b;
        when 6 => e := a xnor b;
        when others => e := b;
      end case;
      assert r = e report "pick" severity failure;
      sum_pick := sum_pick + to_integer(unsigned(r));
      sum_pick2 := sum_pick2 + to_integer(unsigned(pick2(op, a, b)));
      -- Without a loop, calling itself.
      assert parity(a & b) = xor (a & b) severity failure;
      if parity(a) = '1' then sum_par := sum_par + 1; end if;
      -- Results that do not follow from the arguments.
      sum_sig := sum_sig + from_sig(k);
      sum_sh := sum_sh + from_shared(k) mod 1000;
      sum_var := sum_var + from_var(k) mod 1000;
      sum_nest := sum_nest + nested(k);
      if i mod 500 = 0 then
        s <= i;
        wait for 1 ns;
      end if;
    end loop;
    -- A function that reports: every call reports.
    a := "00001010";
    for i in 0 to 3 loop
      sum_ones := sum_ones + noisy(a(3 downto 0));
    end loop;
    file_open(f, "memo_pure_calls.txt", read_mode);
    -- The same call reads the next line each time.
    while not endfile(f) loop
      fv := rd(f);
      sum_file := sum_file * 2 + fv;
    end loop;
    file_close(f);
    report "ones=" & integer'image(sum_ones) & " pick=" & integer'image(sum_pick)
      & " pick2=" & integer'image(sum_pick2) & " par=" & integer'image(sum_par)
      & " sig=" & integer'image(sum_sig)
      & " shared=" & integer'image(sum_sh) & " var=" & integer'image(sum_var)
      & " nest=" & integer'image(sum_nest) & " file=" & integer'image(sum_file);
    wait;
  end process;
end;
