// Copyright 2020 Google LLC
//
// Use of this source code is governed by an MIT-style license that can be found
// in the LICENSE file or at https://opensource.org/licenses/MIT.

mod demand;
mod error;
mod response;
pub mod sink;

use std::convert::TryInto;

use log::error;

use crate::action;
use crate::message;
pub use self::demand::{Demand, Header, Payload};
pub use self::error::{Error, ParseError};
use self::response::{Response, Status};
pub use self::sink::{Sink};

pub type Result<T> = std::result::Result<T, Error>;

pub fn execute<S, R, H>(session: &mut S, handler: H, payload: Payload) -> Result<()>
where
    S: Session,
    R: action::Request,
    H: FnOnce(&mut S, R) -> Result<()>,
{
    handler(session, payload.parse()?)
}

pub fn handle<M>(message: M)
where
    M: TryInto<Demand, Error=ParseError>,
{
    let demand = match message.try_into() {
        Ok(demand) => demand,
        Err(error) => {
            error!("failed to parse the message: {}", error);
            return;
        }
    };

    let mut session = Action::from_demand(&demand);
    let result = action::dispatch(&demand.action, &mut session, demand.payload);

    let status = Status {
        session_id: demand.header.session_id,
        request_id: demand.header.request_id,
        result: result,
    };

    let message = match status.try_into() {
        Ok(message) => message,
        Err(error) => {
            // If we cannot encode the final status message, there is nothing
            // we can do to notify the server, as status is responsible for
            // reporting errors. We can only log the error and carry on.
            error!("failed to encode status message: {}", error);
            return;
        }
    };

    message::send(message);
}

pub trait Session {
    fn reply<R: action::Response>(&mut self, response: R) -> Result<()>;
    fn send<R: action::Response>(&mut self, sink: Sink, response: R) -> Result<()>;
}

pub struct Adhoc;

impl Session for Adhoc {

    // TODO: Session trait should be probably split into two traits and then
    // make the actions that do not care about the `reply` method implement the
    // simpler one.
    fn reply<R>(&mut self, response: R) -> Result<()>
    where
        R: action::Response,
    {
        error!("attempted to reply to an ad-hoc session, dropping response");
        drop(response);

        Ok(())
    }

    fn send<R>(&mut self, sink: Sink, response: R) -> Result<()>
    where
        R: action::Response,
    {
        sink.wrap(response).send()?;

        Ok(())
    }
}

pub struct Action {
    header: Header,
    next_response_id: u64,
}

impl Action {

    pub fn from_demand(demand: &Demand) -> Action {
        Action {
            header: demand.header.clone(),
            next_response_id: 0,
        }
    }

    fn wrap<R>(&self, response: R) -> Response<R>
    where
        R: action::Response
    {
        Response {
            session_id: self.header.session_id.clone(),
            request_id: Some(self.header.request_id),
            response_id: Some(self.next_response_id),
            data: response,
        }
    }
}

impl Session for Action {

    fn reply<R: action::Response>(&mut self, response: R) -> Result<()> {
        self.wrap(response).send()?;
        self.next_response_id += 1;

        Ok(())
    }

    fn send<R>(&mut self, sink: Sink, response: R) -> Result<()>
    where
        R: action::Response,
    {
        sink.wrap(response).send()?;

        Ok(())
    }
}

