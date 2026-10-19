-- LRM 2019 6.5.7.2: a formal package generic map with <> for one generic only.
package g is
  generic (a : integer; b : integer);
end package;
package user is
  generic (package p is new work.g generic map (a => 1, b => <>));
end package;
