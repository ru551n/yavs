-- A group declaration can be decorated with an attribute (LRM 7.2).
package top is
  signal s : bit;
  group one is (signal);
  group g : one (s);
  attribute tag : integer;
  attribute tag of g : group is 3;
end package;
