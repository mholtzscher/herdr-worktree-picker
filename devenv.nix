{ inputs, pkgs, ... }:

{
  packages = with pkgs; [
    gh
    git
  ];

  languages.rust = {
    enable = true;
    channel = "stable";
  };
}
