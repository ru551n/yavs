-- yavs-test: std=2008
-- expect: E1334 11:31 array values under range choices of different directions
-- LRM 9.3.3.3: the aggregate takes the direction of its range choices with array values
-- (lrm-checks L84), so they must agree.
entity e is end entity;
architecture a of e is
begin
  process
    variable v : bit_vector(3 downto 0);
  begin
    v := (1 downto 0 => "01", 2 to 3 => "10");
    wait;
  end process;
end architecture;
