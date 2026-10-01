# Arc capacity harness — convenience over ./arcfleet.
#
#   make up N=1 SIZE=small REGION=uk    bring up N independent Arcs
#   make ls                             show the Arcs
#   make bootstrap                      run arc-node-bootstrap.sh on every Arc
#   make down                           tear it all down (releases the static IPs)
.PHONY: up ls bootstrap down

N      ?= 1
SIZE   ?= small
REGION ?= uk
IMAGE  ?= ubuntu-24-04-loaded

up:
	./arcfleet up --count $(N) --size $(SIZE) --region $(REGION) --image $(IMAGE)

ls:
	./arcfleet ls

bootstrap:
	./arcfleet bootstrap all

down:
	./arcfleet down
