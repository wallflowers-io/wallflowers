These are the  objects used throughout the app. Each object is an MLS group with application layer schema enforcement 
    *.members: The 1:n members of the MLS group
    *.feed: The ordered list of (member, timestamp, Delta) events synchronised for this object via MLS. Stored in LocalStore/Row
    *.state: - the head state of the compiled feature tree of deltas for this object. Cached in LocalStore/Row
    *.schema: - the definition of legal Deltas for instances of this object
    *.vectormap: - the vectorisation protocol for instances this particular object
    *.objective: - the plain-language objective for this type of object.