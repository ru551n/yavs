-- yavs-test: std=2008
-- expect: none
-- LRM 9.3, 5.2.3: the minus applies to a universal_integer literal
package p is
  type big is range -9223372036854775808 to 9223372036854775807;
  type tt is range -9223372036854775808 to 9223372036854775807
    units
      xs;
      ys = 1000 xs;
    end units;
  constant a : big := -9223372036854775808;
  constant b : tt := -9223372036854775808 xs;
  constant c : time := -9223372036854775808 fs;
  constant d : tt := -9223372036854775 ys;
end package;
