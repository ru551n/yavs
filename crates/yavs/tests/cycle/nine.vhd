-- Every pair of the nine std_ulogic values through the logical operators, comparisons,
-- conversions and matches of a combinational process and a register without reset, against the
-- normal mode (the test runs it with and without --cycle; without --two-state the region keeps
-- all nine values).
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_nine is
end entity;
architecture a of cycle_nine is
  signal clk : std_ulogic := '0';
  signal x, y : std_ulogic;
  signal v : std_ulogic_vector(1 downto 0);
  signal g : std_ulogic_vector(0 to 8);
  signal c : std_ulogic_vector(0 to 9);
  signal t : std_ulogic_vector(0 to 6);
  signal r : std_ulogic_vector(0 to 5);
  signal q, q2 : std_ulogic;
  signal qv : std_ulogic_vector(1 downto 0);
  signal sel : std_ulogic_vector(2 downto 0);
  signal bb : bit;
  signal o1 : std_ulogic_vector(1 downto 0);
  signal qb : bit;
  signal qo : boolean;
  signal bo : boolean;
  signal vq : bit_vector(0 to 10);
  signal vo : boolean;
begin
  gates : process (all)
  begin
    g <= (x and y) & (x or y) & (x xor y) & (x nand y) & (x nor y) & (x xnor y) & (not x)
      & (x and '1') & (y or '0');
  end process;
  cmps : process (all)
  begin
    c <= (others => '0');
    if x = y then c(0) <= '1'; end if;
    if x /= y then c(1) <= '1'; end if;
    if x = '1' then c(2) <= '1'; end if;
    if x = 'U' then c(3) <= '1'; end if;
    if x then c(4) <= '1'; end if;
    if std_match(x, y) then c(5) <= '1'; end if;
    if is_x(x) then c(6) <= '1'; end if;
    c(7) <= x ?= y;
    c(8) <= x ?/= y;
    if v = "1H" then c(9) <= '1'; end if;
  end process;
  convs : process (all)
  begin
    t <= to_x01(x) & to_ux01(x) & to_x01z(x) & to_stdulogic(to_bit(x)) & x & y & 'Z';
    bb <= to_bit(y);
    bo <= x = 'H';
    o1 <= to_01(v);
  end process;
  reds : process (all)
  begin
    r <= (and v) & (or v) & (xor v) & (nand v) & (nor v) & (xnor v);
  end process;
  cases : process (all)
  begin
    case x is
      when '0' | 'L' => sel <= "001";
      when '1' => sel <= "010";
      when 'H' => sel <= "011";
      when 'Z' => sel <= "1ZX";
      when 'U' => sel <= "UUU";
      when others => sel <= "-X-";
    end case;
  end process;
  regs : process (clk)
    -- BIT and BOOLEAN variables without an initial value start at '0' and false.
    variable vb : bit;
    variable vo : boolean;
  begin
    if rising_edge(clk) then
      vb := not vb;
      vo := not vo;
      qb <= vb;
      qo <= vo;
      q <= x and y;
      if x = '1' then
        q2 <= y;
      end if;
      qv <= v;
    end if;
  end process;
  -- Wide, record and array-of-record variables of BIT and BOOLEAN also start at '0' and false.
  vregs : process (clk)
    type rb is record
      b : bit;
      o : boolean;
      v : bit_vector(2 downto 0);
    end record;
    type rbs is array (0 to 1) of rb;
    variable wv : bit_vector(69 downto 0);
    variable rv : rb;
    variable av : rbs;
  begin
    if rising_edge(clk) then
      wv := not wv;
      rv.b := not rv.b;
      rv.o := not rv.o;
      rv.v := not rv.v;
      av(1).b := not av(1).b;
      av(1).o := not av(1).o;
      av(0).v := not av(0).v;
      vq <= wv(69) & wv(64) & wv(0) & rv.b & rv.v & av(1).b & av(0).v;
      vo <= rv.o and av(1).o;
    end if;
  end process;
  main : process
    constant all9 : std_ulogic_vector(0 to 8) := "UX01ZWLH-";
  begin
    for i in all9'range loop
      for j in all9'range loop
        x <= all9(i);
        y <= all9(j);
        v <= all9(j) & all9(i);
        wait for 1 ns;
        clk <= '1';
        wait for 1 ns;
        clk <= '0';
        report "x=" & to_string(x) & " y=" & to_string(y) & " g=" & to_string(g) & " c="
          & to_string(c) & " t=" & to_string(t) & " r=" & to_string(r) & " sel=" & to_string(sel)
          & " q=" & to_string(q) & " q2=" & to_string(q2) & " qv=" & to_string(qv) & " bb="
          & bit'image(bb) & " bo=" & boolean'image(bo) & " o1=" & to_string(o1)
          & " qb=" & bit'image(qb) & " qo=" & boolean'image(qo) & " vq=" & to_string(vq)
          & " vo=" & boolean'image(vo);
      end loop;
    end loop;
    wait;
  end process;
end architecture;
