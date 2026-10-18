-- yavs-test: std=2008
-- expect: E1310 19:17 formal `x` is associated twice
-- expect: E1410 20:8 formal index not locally static
-- expect: E1301 21:5 element actual of the wrong type
-- LRM 6.5.7.1
-- A formal is associated either as a whole or by its sub-elements, a formal
-- sub-element name is locally static, and each actual has the type of its sub-element.
entity e is end entity;
architecture a of e is
  signal tl : bit := '0';
  signal bv : bit_vector(0 to 0);
  signal iv : integer;
  procedure pr(signal x : out bit_vector) is begin x(x'left) <= '1'; end procedure;
begin
  process
    variable i : natural := 0;
  begin
    -- whole and part
    pr(x => bv, x(0) => tl);
    pr(x(i) => tl);
    pr(x(0) => iv);
    wait;
  end process;
end architecture;
