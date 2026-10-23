-- yavs-test: std=2008
-- expect: none
-- LRM 5.2.1, 5.3.2.2, 9.3.6: inside, null and run-time ranges are not checked at analysis (#465)
entity e is end;
architecture a of e is
  type ivec is array (integer range <>) of bit;
  constant n : integer := 3;
begin
  process
    variable a : positive range 3 to 1;
    variable b : bit_vector(0 downto 1);
    variable c : natural range 0 to 5;
    variable d : ivec(1 downto -1);
    variable v : ivec(2 downto 0) := "101";
    variable f : bit_vector(n downto 0);
  begin
    report to_string(bit_vector(v));
    wait;
  end process;
end;
