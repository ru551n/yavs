-- yavs-test: std=2008
-- expect: E0001 14:15 an underline in an identifier must stand between two letters or digits
-- expect: E0001 14:21 an underline in an identifier must stand between two letters or digits
-- expect: E0001 14:37 an underline in a numeric literal must stand between two letters or digits
-- expect: E0001 15:42 an underline in a bit string literal must stand between two of its digits
-- expect: E0001 16:46 an underline in a bit string literal must stand between two of its digits
-- expect: E0001 17:51 an underline in a bit string literal must stand between two of its digits
-- LRM 15.4.1, 15.5, 15.8: an underline stands between two letters or digits, or two digits of a bit value (#807)
entity e is
end entity;
architecture a of e is
begin
  process
    variable k__k, k_ : integer := 1__0;
    variable b : bit_vector(1 to 8) := b"_0101_0101";
    variable c : bit_vector(1 to 8) := b"0101__0101";
    variable d : bit_vector(1 to 8) := b"0101_0101_";
    variable ok_1 : bit_vector(1 to 8) := x"A_5";
  begin
    wait;
  end process;
end architecture;
