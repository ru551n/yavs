-- yavs-test: std=2008
-- expect: E1212 11:14 allocator needs a fully constrained subtype, `string` is unconstrained
-- LRM 9.3.7
entity e is end entity;
architecture a of e is
begin
  process
    type sp is access string;
    variable p : sp;
  begin
    p := new string;
    wait;
  end process;
end architecture;
