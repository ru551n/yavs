-- An entry with <> may have no constituent at all (LRM 6.10 is silent on zero).
package top is
  signal s : bit;
  group some is (signal, signal <>);
  group g : some (s);
end package;
