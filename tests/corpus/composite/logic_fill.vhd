-- difftest: top=logic_fill std=2008
-- Logic arrays filled with one value (#444): record defaults, `others` aggregates and
-- defaults of array variables, at lengths around the 64-element plane words.
library ieee;
use ieee.std_logic_1164.all;

entity logic_fill is
end entity;

architecture a of logic_fill is
  type rec is record
    a : std_logic_vector(0 to 0);
    b : std_logic_vector(63 downto 0);
    c : std_logic_vector(64 downto 0);
    d : std_logic_vector(10 downto 4);
    e : bit_vector(1 to 130);
    f : std_logic_vector(1 to 0);
  end record;

  function ones (n : natural) return std_logic_vector is
    variable v : std_logic_vector(1 to n) := (others => '1');
  begin
    return v;
  end function;

  function zs (n : natural) return std_logic_vector is
    variable v : std_logic_vector(n - 1 downto 0);
  begin
    v := (others => 'Z');
    return v;
  end function;

  function count1 (v : std_logic_vector) return integer is
    variable c : integer := 0;
  begin
    for i in v'range loop
      if v(i) = '1' then
        c := c + 1;
      end if;
    end loop;
    return c;
  end function;

  function countb (v : bit_vector) return integer is
    variable c : integer := 0;
  begin
    for i in v'range loop
      if v(i) = '1' then
        c := c + 1;
      end if;
    end loop;
    return c;
  end function;

  type nat_vec is array (natural range <>) of natural;
  constant ns : nat_vec := (0, 1, 63, 64, 65, 127, 128, 129);
  function name4 (v : std_logic_vector(3 downto 0)) return string is
  begin
    case v is
      when "0000"  => return "zero";
      when X"F"    => return "ones";
      when "UUUU"  => return "uninit";
      when "ZZZZ" | "----" => return "special";
      when others  => return "other";
    end case;
  end function;

  function name_bits (v : bit_vector) return string is
  begin
    if v = "101" then
      return "five";
    elsif v = "" then
      return "empty";
    elsif v = "10101" then
      return "twentyone";
    end if;
    return "?";
  end function;

  signal s : std_logic_vector(99 downto 0) := (others => 'H');
begin
  process
    variable r : rec;
    variable big : std_logic_vector(200 downto 1);
    variable w : bit_vector(70 downto 0) := (others => '1');
    variable l : std_logic_vector(5 downto 0) := (others => '-');
  begin
    assert r.a(0) = 'U' and r.b = (63 downto 0 => 'U') and r.c(64) = 'U' and r.c(0) = 'U'
      report "record defaults" severity failure;
    assert r.d = "UUUUUUU" and r.e'length = 130 and countb(r.e) = 0 and r.f'length = 0
      report "record defaults 2" severity failure;
    r.b := (others => '1');
    r.c := (others => 'X');
    r.e := (others => '1');
    r.b(3 downto 0) := (others => '0');
    assert count1(r.b) = 60 and r.c(10) = 'X' and countb(r.e) = 130 report "assign" severity failure;
    for k in ns'range loop
      report "ones " & integer'image(ns(k)) & " " & integer'image(count1(ones(ns(k))));
      assert count1(ones(ns(k))) = ns(k) severity failure;
      assert zs(ns(k) + 1) = (ns(k) downto 0 => 'Z') severity failure;
    end loop;
    assert big = (200 downto 1 => 'U') report "default array" severity failure;
    big := (others => '0');
    big(100 downto 90) := (others => '1');
    assert count1(big) = 11 report "slice" severity failure;
    assert countb(w) = 71 and l = "------" and s = (99 downto 0 => 'H') report "others" severity failure;
    w(64 downto 1) := (others => '0');
    assert countb(w) = 7 report "bits" severity failure;
    report name4("0000") & " " & name4("1111") & " " & name4("UUUU") & " " & name4("ZZZZ")
      & " " & name4("----") & " " & name4("01XZ") & " " & name4("0001");
    report name_bits("101") & " " & name_bits("") & " " & name_bits("10101") & " "
      & name_bits("1010") & " " & name_bits("11111");
    assert (std_logic_vector'("01") & std_logic_vector'("10")) = std_logic_vector'("0110") and not (l = "-----") and (big(3 downto 1) = "000")
      report "equality" severity failure;
    report "done";
    wait;
  end process;
end architecture;
