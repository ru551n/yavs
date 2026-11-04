-- difftest: top=frame_objects_long_block
-- Composite variables of a process in its frame, one long block (#1059): more facts than the
-- optimiser keeps, whole copies and wide slices between two variables and inside one, and a value
-- read long after it was stored.
library ieee;
use ieee.std_logic_1164.all;

entity frame_objects_long_block is
end entity;

architecture a of frame_objects_long_block is
  type rec_t is record
    b : std_ulogic;
    n : integer;
    v : std_ulogic_vector(9 downto 0);
  end record;
  type ints_t is array (0 to 7) of integer;
  signal clk : std_ulogic := '0';
begin
  clk <= not clk after 5 ns;
  process (clk)
    subtype v16 is std_ulogic_vector(15 downto 0);
    subtype v300 is std_ulogic_vector(299 downto 0);
    variable x0 : v16 := x"1234";
    variable x1, x2, x3, x4, x5, x6, x7, x8, x9, x10, x11, x12 : v16 := x"0000";
    variable y0 : v300 := (0 => '1', 77 => '1', 130 => '1', 299 => '1', others => '0');
    variable y1 : v300 := (others => '0');
    variable r0 : rec_t := ('1', 5, "1010101010");
    variable r1, r2 : rec_t := ('0', 0, "0000000000");
    variable i0 : ints_t := (1, 2, 3, 4, 5, 6, 7, 8);
    variable i1 : ints_t := (others => 0);
    variable n : natural := 0;
    variable sum : natural := 0;
  begin
    if rising_edge(clk) then
      x1(7 downto 0) := x0(11 downto 4);
      x2(7 downto 0) := x1(7 downto 0);
      x3(15 downto 8) := x2(7 downto 0);
      x4(11 downto 4) := x3(15 downto 8);
      x5(7 downto 0) := x4(11 downto 4);
      x6(7 downto 0) := x5(7 downto 0);
      x7(15 downto 8) := x6(7 downto 0);
      x8(11 downto 4) := x7(15 downto 8);
      x9(7 downto 0) := x8(11 downto 4);
      x10(7 downto 0) := x9(7 downto 0);
      x11(15 downto 8) := x10(7 downto 0);
      x12(11 downto 4) := x11(15 downto 8);
      -- Inside one variable, overlapping.
      x12(7 downto 0) := x12(11 downto 4);
      y1 := y0;
      y1(299 downto 4) := y0(295 downto 0);
      y0(299 downto 1) := y0(298 downto 0);
      y0(0) := y1(299);
      r1 := r0;
      r1.n := r1.n + n;
      r2 := r1;
      r2.v(4 downto 0) := r1.v(9 downto 5);
      r0.b := not r2.b;
      i1 := i0;
      i1(n mod 8) := i1((n + 1) mod 8) + r2.n;
      i0 := i1;
      x0 := x0(14 downto 0) & x12(7);
      n := n + 1;
      for k in y1'range loop
        if y1(k) = '1' then
          sum := (sum + k) mod 65536;
        end if;
      end loop;
      if n = 40 then
        report to_hstring(x0) & " " & to_hstring(x12) & " " & to_hstring(y0(99 downto 0))
          & " " & to_hstring(y1(299 downto 200)) & " " & std_ulogic'image(r0.b) & " "
          & integer'image(r2.n) & " " & to_hstring(r2.v) & " " & integer'image(i0(3)) & " "
          & integer'image(i1(7)) & " " & integer'image(sum);
        std.env.finish;
      end if;
    end if;
  end process;
end architecture;
