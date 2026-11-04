-- difftest: top=frame_objects_copies
-- Variables stay across the switch of a process to its specialised copy: instances differ by a
-- generic, each with its own fixed record variable and one whose size the generic sets.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity fo_unit is
  generic (k : positive; w : positive);
  port (clk : in std_ulogic; q : out integer);
end entity;

architecture a of fo_unit is
  type fixed_t is record
    n : integer;
    h : std_ulogic_vector(31 downto 0);
    t : boolean;
  end record;
  type sized_t is record
    n : integer;
    h : std_ulogic_vector(w - 1 downto 0);
  end record;
begin
  process (clk)
    variable f : fixed_t := (k, (others => '0'), false);
    variable s : sized_t := (0, (others => '0'));
    variable a : std_ulogic_vector(7 downto 0) := x"01";
  begin
    if rising_edge(clk) then
      f.n := (f.n * 3 + k) mod 1000003;
      f.h := std_ulogic_vector(unsigned(f.h) + to_unsigned(f.n mod 256, 32));
      f.t := not f.t;
      s.n := s.n + 1;
      s.h := std_ulogic_vector(unsigned(s.h) + 1);
      a := a(6 downto 0) & a(7);
      q <= (f.n + s.n + to_integer(unsigned(s.h)) + to_integer(unsigned(a))
            + to_integer(unsigned(f.h(15 downto 0)))) mod 1000003;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity frame_objects_copies is
end entity;

architecture a of frame_objects_copies is
  signal clk : std_ulogic := '0';
  signal done : boolean := false;
  type ints is array (0 to 3) of integer;
  signal q : ints;
begin
  clk <= not clk after 5 ns when not done;
  u0 : entity work.fo_unit generic map (1, 4) port map (clk, q(0));
  u1 : entity work.fo_unit generic map (2, 4) port map (clk, q(1));
  u2 : entity work.fo_unit generic map (3, 9) port map (clk, q(2));
  u3 : entity work.fo_unit generic map (5, 17) port map (clk, q(3));
  process
  begin
    for i in 1 to 4 loop
      for j in 1 to 2500 loop
        wait until rising_edge(clk);
      end loop;
      wait for 1 ns;
      report integer'image(q(0)) & " " & integer'image(q(1)) & " " & integer'image(q(2)) & " "
        & integer'image(q(3));
    end loop;
    done <= true;
    wait;
  end process;
end architecture;
