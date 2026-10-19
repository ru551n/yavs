-- Constituents of different entity classes, in the order of the template (LRM 6.10).
package top is
  signal s : bit;
  constant c : integer := 1;
  group pair is (signal, constant);
  group g : pair (s, c);
end package;
