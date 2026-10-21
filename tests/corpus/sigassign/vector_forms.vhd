-- difftest: top=tb_vector_forms std=2008
-- Assignments of vectors and scalars in clocked processes: a shift register built with `&`, an xor
-- feedback, a vector copied from a variable, delayed and transport forms, signals of bit vectors,
-- unsigned values, integer arrays and records, aggregate targets, and the numeric_std logical
-- operators on vectors with metavalues and null ranges.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb_vector_forms is end;

architecture t of tb_vector_forms is
  type int_arr is array (0 to 3) of integer;
  type rec is record
    a : std_logic_vector(3 downto 0);
    b : integer;
  end record;
  signal clk   : std_logic := '0';
  signal sr    : std_logic_vector(7 downto 0) := x"01";
  signal fb    : std_logic_vector(15 downto 0) := x"ACE1";
  signal cp    : std_logic_vector(7 downto 0) := (others => '0');
  signal bv    : bit_vector(5 downto 0) := "000001";
  signal un    : unsigned(7 downto 0) := x"0F";
  signal sg    : signed(7 downto 0) := x"F0";
  signal ia    : int_arr := (1, 2, 3, 4);
  signal rc    : rec := (a => "0001", b => 5);
  signal tr    : std_logic_vector(3 downto 0) := "0000";
  signal dl    : std_logic_vector(3 downto 0) := "0000";
  signal hi, lo : std_logic_vector(3 downto 0) := "0000";
  signal bit1  : std_logic := '0';
  signal bit2  : std_logic := '0';
  signal cnt   : integer := 0;
  signal done  : boolean := false;
begin
  clk <= not clk after 5 ns when not done;

  -- A shift register with the incoming bit from a counter.
  process (clk)
  begin
    if rising_edge(clk) then
      sr <= sr(6 downto 0) & std_logic'(to_unsigned(cnt, 4)(0));
    end if;
  end process;

  -- An xor feedback shift register on a signal.
  process (clk)
  begin
    if rising_edge(clk) then
      fb <= fb(14 downto 0) & (fb(15) xor fb(13) xor fb(12) xor fb(10));
    end if;
  end process;

  -- The same from a variable, copied to a signal.
  process (clk)
    variable v : std_logic_vector(7 downto 0) := x"A5";
  begin
    if rising_edge(clk) then
      v := v(6 downto 0) & (v(7) xor v(5));
      cp <= v;
    end if;
  end process;

  -- Bit vector, unsigned, signed, integer array and record targets.
  process (clk)
  begin
    if rising_edge(clk) then
      bv <= bv(4 downto 0) & not bv(5);
      un <= un(6 downto 0) & un(7);
      sg <= sg(6 downto 0) & '1';
      ia <= (ia(3), ia(0), ia(1), ia(2) + cnt);
      rc <= (a => rc.a(2 downto 0) & rc.a(3), b => rc.b + 1);
    end if;
  end process;

  -- Transport and delayed assignments of a vector.
  process (clk)
  begin
    if rising_edge(clk) then
      tr <= transport std_logic_vector(to_unsigned(cnt, 4)) after 3 ns;
      dl <= std_logic_vector(to_unsigned(cnt, 4)) after 2 ns;
    end if;
  end process;

  -- An aggregate target and scalar assignments with and without delay.
  process (clk)
  begin
    if rising_edge(clk) then
      (hi, lo) <= std_logic_vector'(x"A" & std_logic_vector(to_unsigned(cnt, 4)));
      bit1 <= not bit1;
      bit2 <= bit1 after 1 ns;
    end if;
  end process;

  -- The checks.
  process
    variable a, b : unsigned(7 downto 0);
    variable s, t : signed(3 downto 0);
    variable n    : unsigned(0 downto 1);
    variable x, y : unsigned(4 downto 0);
    variable m    : unsigned(0 downto 1);
    variable r    : unsigned(7 downto 0);
  begin
    for i in 1 to 12 loop
      wait until rising_edge(clk);
      cnt <= cnt + 1;
    end loop;
    wait for 1 ns;
    report "sr=" & to_string(sr) & " fb=" & to_string(fb) & " cp=" & to_string(cp);
    report "bv=" & to_string(bv) & " un=" & to_string(std_logic_vector(un)) & " sg=" & to_string(std_logic_vector(sg));
    report "ia=" & integer'image(ia(0)) & "," & integer'image(ia(1)) & "," & integer'image(ia(2)) & "," & integer'image(ia(3));
    report "rc=" & to_string(rc.a) & "," & integer'image(rc.b);
    report "tr=" & to_string(tr) & " dl=" & to_string(dl) & " hi=" & to_string(hi) & " lo=" & to_string(lo);
    report "bit1=" & to_string(bit1) & " bit2=" & to_string(bit2);
    -- numeric_std logical operators.
    a := "10101010";
    b := "11001100";
    report "and=" & to_string(a and b) & " or=" & to_string(a or b) & " xor=" & to_string(a xor b);
    report "nand=" & to_string(a nand b) & " nor=" & to_string(a nor b) & " xnor=" & to_string(a xnor b);
    report "not=" & to_string(not a);
    s := "01XZ";
    t := "1-WL";
    report "meta and=" & to_string(s and t) & " xor=" & to_string(s xor t) & " not=" & to_string(not t);
    x := "0UH10";
    y := "1LZ01";
    report "meta or=" & to_string(x or y) & " xnor=" & to_string(x xnor y);
    m := n and n;
    r := a xor b;
    report "null and=" & integer'image(m'length) & " not=" & integer'image(m'left);
    m := not n;
    report "null not=" & integer'image(m'length) & " " & integer'image(m'left) & " " & integer'image(m'right);
    report "bounds=" & integer'image(r'left) & " " & integer'image(r'right);
    done <= true;
    wait;
  end process;
end;
