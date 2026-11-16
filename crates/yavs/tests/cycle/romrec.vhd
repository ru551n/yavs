-- A constant table of records with enumeration, integer, logic and vector fields, indexed by an
-- enumeration chosen at run time and assigned whole to a record register (a decoder ROM), against
-- the normal mode (`W` replaced by a width).
library ieee;
use ieee.std_logic_1164.all;
entity cycle_romrec is
end entity;
architecture t of cycle_romrec is
  type op_t is (op_a, op_b, op_c, op_d, op_e);
  type unit_t is (none, alu, ldst, br);
  type row_t is record
    unit : unit_t;
    lat : natural range 0 to 7;
    rd : std_ulogic;
    sel : std_ulogic_vector(W - 1 downto 0);
  end record;
  type rom_t is array (op_t) of row_t;
  constant ROM : rom_t := (
    op_a => (alu, 1, '1', (others => '0')),
    op_b => (ldst, 3, '0', (others => '1')),
    op_c => (br, 2, '1', (0 => '1', others => '0')),
    op_d => (none, 0, '0', (W - 1 => '1', others => '0')),
    op_e => (alu, 7, '1', (others => '1')));
  signal clk : std_ulogic := '0';
  signal addr : op_t := op_a;
  signal row : row_t := (none, 0, '0', (others => '0'));
begin
  decode : process (clk)
  begin
    if rising_edge(clk) then
      row <= ROM(addr);
    end if;
  end process;
  main : process
  begin
    for k in 1 to 12 loop
      addr <= op_t'val((k * 3) mod 5);
      wait for 1 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 4 ns;
      report "k=" & integer'image(k) & " unit=" & unit_t'image(row.unit) & " lat="
        & integer'image(row.lat) & " rd=" & std_ulogic'image(row.rd) & " sel=" & to_hstring(row.sel);
    end loop;
    wait;
  end process;
end architecture;
