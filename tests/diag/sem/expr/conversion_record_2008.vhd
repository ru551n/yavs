-- yavs-test: std=2008
-- expect: E1350 13:23 cannot convert `b_t` to `a_t`
-- LRM 9.3.6 (closely related records are new in 2019)
package p is
  type a_t is record
    x : bit_vector(7 downto 0);
    r : integer;
  end record;
  type b_t is record
    x : bit_vector(7 downto 0);
    r : integer;
  end record;
  constant c : a_t := a_t(b_t'(x => x"42", r => 4));
end package;
