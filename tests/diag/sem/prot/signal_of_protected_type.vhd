-- yavs-test: std=2008
-- expect: E1905 8:10 a signal cannot have protected type `pt`
-- LRM 5.6.2
package p is
  type pt is protected
    procedure set (x : integer);
  end protected;
  signal s : pt;
end package;
