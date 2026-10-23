-- difftest: top=record_constant_element_bounds std=2019
-- A record constant with open fields takes the field bounds of its subtype, not those of its
-- initial value, and a signal initialised from it keeps them: a folded initial value and one a
-- function call computes.
entity record_constant_element_bounds is end entity;

architecture a of record_constant_element_bounds is
  type t_rec is record
    f, g : bit_vector;
  end record;

  function make return t_rec is
  begin
    return ("1010", "111");
  end function;

  constant folded : t_rec(f(10 to 13), g(0 to 2)) := ("1010", "111");
  constant called : t_rec(f(4 downto 1), g(7 to 9)) := make;

  signal s1 : t_rec(g(20 to 22)) := folded;
  signal s2 : t_rec := folded;
  signal s3 : t_rec(g(3 to 5)) := called;
begin
  process
  begin
    assert folded.f'left = 10 and folded.f'right = 13 report "folded f" severity failure;
    assert s1.f'left = 10 and s1.f'right = 13 report "s1 f" severity failure;
    assert s1.g'left = 20 and s1.g'right = 22 report "s1 g" severity failure;
    assert s2.f'left = 10 and s2.g'left = 0 and s2.g'right = 2 report "s2" severity failure;
    assert s3.f'left = 4 and s3.f'right = 1 report "s3 f" severity failure;
    assert s3.g'left = 3 and s3.g'right = 5 report "s3 g" severity failure;
    report "ok " & to_string(s1.f) & to_string(s1.g) & " " & to_string(s3.f) & to_string(s3.g);
    wait;
  end process;
end architecture;
