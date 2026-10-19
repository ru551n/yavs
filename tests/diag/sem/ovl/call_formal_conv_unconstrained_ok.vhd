-- yavs-test: std=2008
-- LRM 5.3.2.2, 6.5.7.1
-- A conversion that defines the bounds is legal on an unconstrained formal.
entity e is end entity;
architecture a of e is
  subtype b4 is bit_vector(1 to 4);
  procedure setv(variable x : out bit_vector) is begin x := (x'range => '1'); end procedure;
begin
  process
    variable b : bit_vector(0 to 3);
  begin
    setv(b4(x) => b);
    wait;
  end process;
end architecture;
