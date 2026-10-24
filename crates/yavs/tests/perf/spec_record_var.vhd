-- Processes with variables of record, array-of-record and unconstrained array types whose bounds come
-- from generics, a constant and a function-made value in the declarative part: a specialised copy
-- (YAVS_SPEC_AFTER) knows the layouts and bounds the declarative part made and folds them.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package spec_record_pk is
  function clog2(n : positive) return natural;
  procedure bump(variable x : inout integer);
end package;
package body spec_record_pk is
  function clog2(n : positive) return natural is
    variable r : natural := 0; variable x : natural := 1;
  begin
    while x < n loop x := x * 2; r := r + 1; end loop;
    return r;
  end function;
  procedure bump(variable x : inout integer) is begin x := (x + 3) mod 1000; end procedure;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.spec_record_pk.all;

entity spec_record_sub is
  generic (N : positive := 4; M : positive := 3; T : natural := 1);
  port (clk : in std_logic; d : in std_logic_vector(N-1 downto 0); q : out std_logic_vector(N-1 downto 0); o : out integer);
end entity;

architecture rtl of spec_record_sub is
  type rec_t is record
    a : std_logic_vector(N-1 downto 0);
    b : unsigned(M-1 downto 0);
    c : integer;
  end record;
  type arr_t is array (0 to M-1) of rec_t;
  signal r, rin : rec_t := (a => (others => '0'), b => (others => '0'), c => 0);
  signal acc : integer := 0;
begin
  comb : process (r, d)
    variable v : rec_t;
    variable w : rec_t;
    variable ar : arr_t;
    constant K2 : integer := N * 2 + T;
    constant L2 : natural := clog2(N);
    variable cnt : integer := 0;
    variable vec : std_logic_vector(N-1 downto 0) := (others => '0');
    variable vec2 : unsigned(M-1 downto 0) := to_unsigned(T + 1, M);
    alias hi : std_logic_vector(N/2 downto 0) is vec(N-1 downto N-1-N/2);
    variable tmp : integer := 7;
  begin
    cnt := (cnt + K2 + L2) mod 1000;
    bump(tmp);
    v := r;
    v.a := d xor vec;
    v.b := v.b + vec2 + 1;
    v.c := (v.c + to_integer(v.b) + cnt + tmp) mod 1000;
    w := v;
    w.a(0) := '1';
    ar(0) := w;
    ar(M-1) := v;
    ar(1 mod M).c := (ar(0).c + 1) mod 1000;
    hi(0) := d(0);
    vec(0) := not vec(0);
    rin <= ar(M-1);
    rin.a(N-1) <= hi(0);
  end process;
  reg : process (clk)
  begin
    if rising_edge(clk) then
      r <= rin;
      acc <= (acc + r.c) mod 100000;
    end if;
  end process;
  q <= r.a;
  o <= acc;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity spec_record_var is end entity;
architecture t of spec_record_var is
  signal clk : std_logic := '0';
  signal d1, q1 : std_logic_vector(7 downto 0);
  signal d2, q2 : std_logic_vector(2 downto 0);
  signal d3, q3 : std_logic_vector(5 downto 0);
  signal o1, o2, o3 : integer;
begin
  clk <= not clk after 5 ns;
  u1 : entity work.spec_record_sub generic map (8, 5, 1) port map (clk, d1, q1, o1);
  u2 : entity work.spec_record_sub generic map (3, 2, 2) port map (clk, d2, q2, o2);
  u3 : entity work.spec_record_sub generic map (6, 4, 0) port map (clk, d3, q3, o3);
  process (clk)
    variable n : natural := 0;
  begin
    if rising_edge(clk) then
      n := n + 1;
      d1 <= std_logic_vector(to_unsigned(n mod 256, 8));
      d2 <= std_logic_vector(to_unsigned(n mod 8, 3));
      d3 <= std_logic_vector(to_unsigned((n * 5) mod 64, 6));
    end if;
    if n = 400 then
      report "sum=" & integer'image(to_integer(unsigned(q1))) & " " & integer'image(to_integer(unsigned(q2))) & " " & integer'image(to_integer(unsigned(q3)))
        & " " & integer'image(o1) & " " & integer'image(o2) & " " & integer'image(o3);
      std.env.stop;
    end if;
  end process;
end architecture;
