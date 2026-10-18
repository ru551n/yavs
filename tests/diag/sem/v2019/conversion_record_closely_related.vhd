-- yavs-test: std=2019
-- expect: E1350 18:24 cannot convert `c_t` to `a_t`
-- LRM 9.3.6 (2019, LCS-2016-075)
package p is
  type a_t is record
    x : bit_vector(7 downto 0);
    r : integer;
  end record;
  type b_t is record
    r : real;
    x : bit_vector(0 to 7);
  end record;
  type c_t is record
    x : bit_vector(7 downto 0);
    y : integer;
  end record;
  constant ab : a_t := a_t(b_t'(x => x"42", r => 4.0));
  constant ac : a_t := a_t(c_t'(x => x"42", y => 4));
end package;
