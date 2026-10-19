-- Objects of every kind vhpi_get_value reads (tests/plugin/values.c).
library ieee;
use ieee.std_logic_1164.all;

entity top is
  generic (g : integer := 5);
  port (p : in std_logic_vector(3 downto 0) := "01ZX");
end entity;

architecture a of top is
  type state_t is (idle, run, done);
  type rec_t is record
    a : std_logic;
    b : integer;
    c : std_logic_vector(3 downto 0);
  end record;
  type mem_t is array (0 to 1, 0 to 2) of integer;
  type dist_t is range 0 to 1000000 units mm; m = 1000 mm; end units;
  signal sl  : std_logic := 'H';
  signal slv : std_logic_vector(7 downto 0) := "10XZ01UH";
  signal sv  : std_logic_vector(0 to 5) := "110010";
  signal bv  : bit_vector(0 to 3) := "1010";
  signal b   : bit := '1';
  signal bo  : boolean := true;
  signal i   : integer := -42;
  signal r   : real := 2.5;
  signal t   : time := 3 ns;
  signal d   : dist_t := 2 m;
  signal st  : state_t := run;
  signal s   : string(1 to 5) := "hello";
  signal ch  : character := 'Q';
  signal rec : rec_t := ('0', 7, "1100");
  signal mem : mem_t := ((1, 2, 3), (4, 5, 6));
  signal iv  : integer_vector(0 to 2) := (10, -20, 30);
  signal rv  : real_vector(0 to 1) := (1.5, -0.25);
  signal cnt : integer := 0;
  constant k : integer := 99;
begin
  process
  begin
    wait for 1 ns;
    cnt <= 17;
    wait;
  end process;
end architecture;
