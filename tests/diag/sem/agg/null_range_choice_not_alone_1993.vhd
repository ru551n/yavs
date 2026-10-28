-- yavs-test: std=1993
-- expect: E1326 14:11 a null range choice must be the only choice
-- expect: E1326 15:20 a null range choice must be the only choice
-- LRM 1993 7.3.2.2: a null range choice only as the single choice of the single association (#812; later revisions are permissive, as sim1 and sim2)
entity e is
end entity;
architecture a of e is
begin
  process
    type t16 is array (1 to 1) of integer;
    variable v : t16;
    variable n : bit_vector(1 to 0);
  begin
    v := (1 to 0 => 16, 1 => 12);
    v := (1 => 12, 2 to 1 => 7);
    n := (1 to 0 => '0');
    wait;
  end process;
end architecture;
