-- A group declaration has as many constituents as its template has entries (LRM 6.10).
package top is
  signal s : bit;
  group two is (signal, signal);
  group g : two (s);
end package;
