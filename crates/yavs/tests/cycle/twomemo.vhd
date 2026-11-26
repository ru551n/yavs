-- --two-state below the top: a decoder function with '-' patterns, called with few distinct
-- arguments (its calls are answered from the memo of pure calls), and records with fields of
-- 1, 63, 64, 65 and 128 elements and a scalar holding all nine values, read whole from a signal
-- (each field and element mapped: '1' and 'H' are 1, every other value 0).
library ieee;
use ieee.std_logic_1164.all;

package twomemo_pkg is
  type rec_t is record
    a : std_ulogic_vector(0 downto 0);
    b : std_ulogic_vector(62 downto 0);
    c : std_ulogic_vector(63 downto 0);
    d : std_ulogic_vector(64 downto 0);
    e : std_ulogic_vector(127 downto 0);
    s : std_ulogic;
  end record;
  type recs_t is array (0 to 2) of rec_t;
end package;

library ieee;
use ieee.std_logic_1164.all;
use work.twomemo_pkg.all;

entity twomemo_dut is
  port (clk : in std_ulogic; insn : in std_ulogic_vector(7 downto 0); rs : in recs_t);
end entity;

architecture rtl of twomemo_dut is
  function match_f(x, p : std_ulogic_vector) return boolean is
    variable m : boolean := true;
  begin
    for i in x'range loop
      if (p(i) = '1') or (p(i) = '0') then
        m := m and (p(i) = x(i));
      end if;
    end loop;
    return m;
  end function;
  type ent_t is record
    pat : std_ulogic_vector(7 downto 0);
    id : natural;
  end record;
  type ents_t is array (0 to 3) of ent_t;
  constant ents : ents_t := (("----0011", 1), ("--1-0001", 2), ("0---1---", 3), ("--------", 4));
  function dec_f(x : std_ulogic_vector(7 downto 0)) return natural is
  begin
    for i in ents'range loop
      if match_f(x, ents(i).pat) then
        return ents(i).id;
      end if;
    end loop;
    return 0;
  end function;
  function ones(v : std_ulogic_vector) return natural is
    variable n : natural := 0;
  begin
    for i in v'range loop
      if v(i) = '1' then
        n := n + 1;
      end if;
    end loop;
    return n;
  end function;
begin
  process (clk)
    variable sum : natural := 0;
    variable r : recs_t;
  begin
    if rising_edge(clk) then
      sum := sum + dec_f(insn);
      r := rs;
      for k in r'range loop
        report "rec" & integer'image(k) & " a=" & integer'image(ones(r(k).a)) & " b="
          & integer'image(ones(r(k).b)) & " c=" & integer'image(ones(r(k).c)) & " d="
          & integer'image(ones(r(k).d)) & " e=" & integer'image(ones(r(k).e)) & " s="
          & std_ulogic'image(r(k).s) & " sum=" & integer'image(sum);
      end loop;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.twomemo_pkg.all;

entity twomemo is
end entity;

architecture tb of twomemo is
  constant nine : std_ulogic_vector(0 to 8) := "UX01ZWLH-";
  -- Element `k` of every field of record `j` holds value `(k + 4 j + 3) mod 9` of the nine.
  function fill(n, j : natural) return std_ulogic_vector is
    variable v : std_ulogic_vector(n - 1 downto 0);
  begin
    for k in 0 to n - 1 loop
      v(k) := nine((k + 4 * j + 3) mod 9);
    end loop;
    return v;
  end function;
  signal clk : std_ulogic := '0';
  signal insn : std_ulogic_vector(7 downto 0) := (others => '0');
  signal rs : recs_t;
begin
  dut : entity work.twomemo_dut port map (clk, insn, rs);
  g : for j in 0 to 2 generate
    rs(j) <= (fill(1, j), fill(63, j), fill(64, j), fill(65, j), fill(128, j), nine((j + 7) mod 9));
  end generate;
  process
  begin
    for i in 0 to 199 loop
      insn <= std_ulogic_vector(to_unsigned((i * 37) mod 6, 8));
      wait for 1 ns;
      clk <= '1';
      wait for 1 ns;
      clk <= '0';
    end loop;
    wait;
  end process;
end architecture;
