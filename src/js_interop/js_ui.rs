extern crate yew;
use yew::prelude::*;
use expression::Expr;
use rules::{Rule, RuleM, RuleT};
use proofs::{Proof, Justification, pooledproof::PooledProof};
use std::collections::{BTreeSet,HashMap};
use std::mem;
use wasm_bindgen::JsCast;

pub struct ExprEntry {
    link: ComponentLink<Self>,
    current_input: String,
    last_good_parse: String,
    current_expr: Option<Expr>,
    onchange: Callback<(String, Option<Expr>)>,
}

#[derive(Clone, Properties)]
pub struct ExprEntryProps {
    pub initial_contents: String,
    pub onchange: Callback<(String, Option<Expr>)>,
}

impl Component for ExprEntry {
    type Message = String;
    type Properties = ExprEntryProps;
    fn create(props: Self::Properties, link: ComponentLink<Self>) -> Self {
        let mut ret = Self {
            link,
            current_expr: None,
            current_input: props.initial_contents.clone(),
            last_good_parse: "".into(),
            onchange: props.onchange,
        };
        ret.update(props.initial_contents);
        ret
    }
    fn update(&mut self, msg: Self::Message) -> ShouldRender {
        use parser::parse;
        self.current_input = msg.clone();
        self.current_expr = parse(&*msg);
        if let Some(expr) = &self.current_expr {
            self.last_good_parse = format!("{}", expr);
        }
        self.onchange.emit((self.last_good_parse.clone(), self.current_expr.clone()));
        true
    }
    fn view(&self) -> Html {
        html! {
            <div>
                <input type="text" oninput=self.link.callback(|e: InputData| e.value) style="width:400px" value={ &self.current_input } />
                <div>
                    { &self.last_good_parse }
                    <br/>
                    <pre>
                        { self.current_expr.as_ref().map(|e| format!("{:#?}", e)).unwrap_or("Error".into()) }
                    </pre>
                </div>
            </div>
        }
    }
}

// yew doesn't seem to allow Components to be generic over <P: Proof>, so fix a proof type P at the module level
pub type P = PooledProof<Hlist![Expr]>;

pub struct ProofUiData<P: Proof> {
    ref_to_line_depth: HashMap<<P as Proof>::Reference, (usize, usize)>,
    ref_to_input: HashMap<<P as Proof>::Reference, String>,
}

impl<P: Proof> ProofUiData<P> {
    pub fn from_proof(prf: &P) -> ProofUiData<P> {
        let mut ref_to_line_depth = HashMap::new();
        calculate_lineinfo::<P>(&mut ref_to_line_depth, prf.top_level_proof(), &mut 1, &mut 0);
        ProofUiData {
            ref_to_line_depth,
            ref_to_input: initialize_inputs(prf),
        }
    }
}

pub struct ProofWidget {
    link: ComponentLink<Self>,
    prf: P,
    pud: ProofUiData<P>,
    selected_line: Option<<P as Proof>::Reference>,
    preblob: String,
    props: ProofWidgetProps,
}

#[derive(Debug)]
pub enum LAKItem {
    Line, Subproof
}

#[derive(Debug)]
pub enum LineActionKind {
    Insert { what: LAKItem, after: bool, relative_to: LAKItem, },
    Delete { what: LAKItem },
    SetRule { rule: Rule },
    Select,
    SetDependency { to: bool, dep: frunk::Coproduct<<P as Proof>::Reference, frunk::Coproduct<<P as Proof>::SubproofReference, frunk::coproduct::CNil>> },
}

#[derive(Debug)]
pub enum ProofWidgetMsg {
    Nop,
    LineChanged(<P as Proof>::Reference, String),
    LineAction(LineActionKind, <P as Proof>::Reference),
}

#[derive(Clone, Properties)]
pub struct ProofWidgetProps {
    verbose: bool,
    blank: bool,
}

impl ProofWidget {
    pub fn render_dep_or_sdep_checkbox(&self, proofref: Coprod!(<P as Proof>::Reference, <P as Proof>::SubproofReference)) -> Html {
        if let Some(selected_line) = self.selected_line {
            use frunk::Coproduct::{Inl, Inr};
            if let Inr(Inl(_)) = selected_line {
                let lookup_result = self.prf.lookup(selected_line.clone()).expect("selected_line should exist in self.prf");
                let just: &Justification<_, _, _> = lookup_result.get().expect("selected_line already is a JustificationReference");
                let checked = match proofref {
                    Inl(lr) => just.2.contains(&lr),
                    Inr(Inl(sr)) => just.3.contains(&sr),
                    Inr(Inr(void)) => match void {},
                };
                let dep = proofref.clone();
                let selected_line_ = selected_line.clone();
                let handle_dep_changed = self.link.callback(move |e: MouseEvent| {
                    if let Some(target) = e.target() {
                        if let Ok(checkbox) = target.dyn_into::<web_sys::HtmlInputElement>() {
                            return ProofWidgetMsg::LineAction(LineActionKind::SetDependency { to: checkbox.checked(), dep }, selected_line_);
                        }
                    }
                    ProofWidgetMsg::Nop
                });
                if self.prf.can_reference_dep(&selected_line, &proofref) {
                    return html! { <input type="checkbox" onclick=handle_dep_changed checked=checked></input> };
                }
            }
        }
        yew::virtual_dom::VNode::from(yew::virtual_dom::VList::new())
    }
    pub fn render_justification_widget(&self, _line: usize, _depth: usize, proofref: <P as Proof>::Reference) -> Html {
        /* TODO: does HTML/do browsers have a way to do nested menus?
        https://developer.mozilla.org/en-US/docs/Web/HTML/Element/menu is 
        "experimental", and currently firefox only, and a bunch of tutorials for the 
        DDG query "javascript nested context menus" build their own menus out of 
        {div,nav,ul,li} with CSS for displaying the submenus on hover */ 
        use frunk::Coproduct::{Inl, Inr};
        if let Inr(Inl(_)) = proofref {
            let proofref_ = proofref.clone();
            let handle_rule_select = self.link.callback(move |e: ChangeData| {
                if let ChangeData::Select(s) = e {
                    if let Some(rule) = RuleM::from_serialized_name(&s.value()) {
                        return ProofWidgetMsg::LineAction(LineActionKind::SetRule { rule }, proofref_);
                    }
                }
                ProofWidgetMsg::Nop
            });
            let lookup_result = self.prf.lookup(proofref.clone()).expect("proofref should exist in self.prf");
            let just: &Justification<_, _, _> = lookup_result.get().expect("proofref already is a JustificationReference");
            let mut dep_lines = String::new();
            for (i, dep) in just.2.iter().enumerate() {
                let (dep_line, _) = self.pud.ref_to_line_depth[&dep];
                dep_lines += &format!("{}{}", dep_line, if i < just.2.len()-1 { ", " } else { "" })
            }
            if just.2.len() > 0 && just.3.len() > 0 {
                dep_lines += "; "
            }
            for (i, sdep) in just.3.iter().enumerate() {
                if let Some(sub) = self.prf.lookup_subproof(sdep.clone()) {
                    let (mut lo, mut hi) = (usize::max_value(), usize::min_value());
                    for line in sub.premises().into_iter().chain(sub.direct_lines().into_iter()) {
                        if let Some((i, _)) = self.pud.ref_to_line_depth.get(&line) {
                            lo = std::cmp::min(lo, *i);
                            hi = std::cmp::max(hi, *i);
                        }
                    }
                    dep_lines += &format!("{}-{}{}", lo, hi, if i < just.3.len()-1 { ", " } else { "" });
                }
            }

            let mut rules = yew::virtual_dom::VList::new();
            for rule in RuleM::ALL_RULES {
                // TODO: seperators and submenus by RuleClassification
                rules.add_child(html!{ <option value=RuleM::to_serialized_name(*rule) selected=(just.1 == *rule)> { rule.get_name() } </option> });
            }
            html! {
                <div>
                <td>
                <select onchange=handle_rule_select>
                    <option value="no_rule_selected">{"Rule"}</option>
                    <hr />
                    { rules }
                </select>
                </td>
                <td><input type="text" readonly=true value=dep_lines></input></td>
                </div>
            }
        } else {
            yew::virtual_dom::VNode::from(yew::virtual_dom::VList::new())
        }
    }
    pub fn render_proof_line(&self, line: usize, depth: usize, proofref: <P as Proof>::Reference) -> Html {
        let selection_indicator =
            if self.selected_line == Some(proofref.clone()) {
                html! { <span style="background-color: cyan; color: blue"> { ">" } </span> }
            } else {
                yew::virtual_dom::VNode::from(yew::virtual_dom::VList::new())
            };
        let dep_checkbox = self.render_dep_or_sdep_checkbox(frunk::Coproduct::inject(proofref.clone()));
        let lineinfo = format!("{}", line);
        let mut indentation = yew::virtual_dom::VList::new();
        for _ in 0..(depth+1) {
            indentation.add_child(html! { <span style="background-color:black">{"-"}</span>});
            indentation.add_child(html! { <span style="color:white">{"-"}</span>});
        }
        let proofref_ = proofref.clone();
        let handle_action = self.link.callback(move |e: ChangeData| {
            if let ChangeData::Select(s) = e {
                let value = s.value();
                s.set_selected_index(0);
                match &*value {
                    "delete_line" => ProofWidgetMsg::LineAction(LineActionKind::Delete { what: LAKItem::Line }, proofref_.clone()),
                    "delete_subproof" => ProofWidgetMsg::LineAction(LineActionKind::Delete { what: LAKItem::Subproof }, proofref_.clone()),
                    "insert_line_before_line" => ProofWidgetMsg::LineAction(LineActionKind::Insert { what: LAKItem::Line, after: false, relative_to: LAKItem::Line }, proofref_.clone()),
                    "insert_line_after_line" => ProofWidgetMsg::LineAction(LineActionKind::Insert { what: LAKItem::Line, after: true, relative_to: LAKItem::Line }, proofref_.clone()),
                    "insert_line_before_subproof" => ProofWidgetMsg::LineAction(LineActionKind::Insert { what: LAKItem::Line, after: false, relative_to: LAKItem::Subproof }, proofref_.clone()),
                    "insert_line_after_subproof" => ProofWidgetMsg::LineAction(LineActionKind::Insert { what: LAKItem::Line, after: true, relative_to: LAKItem::Subproof }, proofref_.clone()),
                    "insert_subproof_before_line" => ProofWidgetMsg::LineAction(LineActionKind::Insert { what: LAKItem::Subproof, after: false, relative_to: LAKItem::Line }, proofref_.clone()),
                    "insert_subproof_after_line" => ProofWidgetMsg::LineAction(LineActionKind::Insert { what: LAKItem::Subproof, after: true, relative_to: LAKItem::Line }, proofref_.clone()),
                    "insert_subproof_before_subproof" => ProofWidgetMsg::LineAction(LineActionKind::Insert { what: LAKItem::Subproof, after: false, relative_to: LAKItem::Subproof }, proofref_.clone()),
                    "insert_subproof_after_subproof" => ProofWidgetMsg::LineAction(LineActionKind::Insert { what: LAKItem::Subproof, after: true, relative_to: LAKItem::Subproof }, proofref_.clone()),
                    _ => ProofWidgetMsg::Nop,
                }
            } else {
                ProofWidgetMsg::Nop
            }
        });
        let proofref_ = proofref.clone();
        let handle_input = self.link.callback(move |e: InputData| ProofWidgetMsg::LineChanged(proofref_.clone(), e.value.clone()));
        let proofref_ = proofref.clone();
        let select_line = self.link.callback(move |_| ProofWidgetMsg::LineAction(LineActionKind::Select, proofref_.clone()));
        let action_selector = {
            use frunk::Coproduct::{self, Inl, Inr};
            let mut options = yew::virtual_dom::VList::new();
            if may_remove_line(&self.prf, &proofref) {
                options.add_child(html! { <option value="delete_line">{ "Delete line" }</option> });
            }
            if let Some(_) = self.prf.parent_of_line(&Coproduct::inject(proofref.clone())) {
                // only allow deleting non-root subproofs
                options.add_child(html! { <option value="delete_subproof">{ "Delete subproof" }</option> });
            }
            match proofref {
                Inl(_) => {
                    options.add_child(html! { <option value="insert_line_before_line">{ "Insert premise before this premise" }</option> });
                    options.add_child(html! { <option value="insert_line_after_line">{ "Insert premise after this premise" }</option> });
                },
                Inr(Inl(_)) => {
                    options.add_child(html! { <option value="insert_line_before_line">{ "Insert step before this step" }</option> });
                    options.add_child(html! { <option value="insert_line_after_line">{ "Insert step after this step" }</option> });
                    // Only show subproof creation relative to justification lines, since it may confuse users to have subproofs appear after all the premises when they selected a premise
                    options.add_child(html! { <option value="insert_subproof_before_line">{ "Insert subproof before this step" }</option> });
                    options.add_child(html! { <option value="insert_subproof_after_line">{ "Insert subproof after this step" }</option> });
                },
                Inr(Inr(void)) => match void {},
            }
            html! {
            <select onchange=handle_action>
                <option value="Action">{ "Action" }</option>
                <hr />
                { options }
                //<option value="insert_line_before_subproof">{ "insert_line_before_subproof" }</option>
                //<option value="insert_line_after_subproof">{ "insert_line_after_subproof" }</option>
                //<option value="insert_subproof_before_subproof">{ "insert_subproof_before_subproof" }</option>
                //<option value="insert_subproof_after_subproof">{ "insert_subproof_after_subproof" }</option>
            </select>
            }
        };
        let justification_widget = self.render_justification_widget(line, depth, proofref.clone());
        let rule_feedback = (|| {
            use parser::parse;
            let raw_line = match self.pud.ref_to_input.get(&proofref).and_then(|x| if x.len() > 0 { Some(x) } else { None }) {
                None => { return html! { <span></span> }; },
                Some(x) => x,
            };
            match parse(&raw_line).map(|_| self.prf.verify_line(&proofref)) {
                None => html! { <span style="background-color:yellow">{ "Parse error" }</span> },
                Some(Ok(())) => html! { <span style="background-color:lightgreen">{ "Correct" }</span> },
                Some(Err(e)) => {
                    // TODO: proper CSS hover box
                    html! { <span style="background-color:red" title=format!("{}", e)>{ "Error (hover for details)" }</span> }
                },
            }
        })();
        html! {
            <tr>
                <td> { selection_indicator } </td>
                <td> { lineinfo } </td>
                <td> { dep_checkbox } </td>
                <td>
                { indentation }
                { action_selector }
                <input type="text" oninput=handle_input onfocus=select_line style="width:400px" value=self.pud.ref_to_input.get(&proofref).unwrap_or(&String::new()) />
                </td>
                { justification_widget }
                <td>{ rule_feedback }</td>
            </tr>
        }
    }

    pub fn render_proof(&self, prf: &<P as Proof>::Subproof, sref: Option<<P as Proof>::SubproofReference>, line: &mut usize, depth: &mut usize) -> Html {
        let mut output = yew::virtual_dom::VList::new();
        for prem in prf.premises() {
            output.add_child(self.render_proof_line(*line, *depth, prem.clone()));
            *line += 1;
        }
        let sdep_checkbox = match sref {
            Some(sr) => self.render_dep_or_sdep_checkbox(frunk::Coproduct::inject(sr)),
            None => yew::virtual_dom::VNode::from(yew::virtual_dom::VList::new()),
        };
        let mut spacer = yew::virtual_dom::VList::new();
        spacer.add_child(html! { <td></td> });
        spacer.add_child(html! { <td></td> });
        spacer.add_child(html! { <td>{ sdep_checkbox }</td> });
        spacer.add_child(html! { <td style="background-color:black"></td> });
        output.add_child(yew::virtual_dom::VNode::from(spacer));
        for lineref in prf.lines() {
            use frunk::Coproduct::{Inl, Inr};
            match lineref {
                Inl(r) => { output.add_child(self.render_proof_line(*line, *depth, r.clone())); *line += 1; },
                Inr(Inl(sr)) => { *depth += 1; output.add_child(self.render_proof(&prf.lookup_subproof(sr.clone()).unwrap(), Some(sr), line, depth)); *depth -= 1; },
                Inr(Inr(void)) => { match void {} },
            }
        }
        if *depth == 0 {
            html! { <table>{ output }</table> }
        } else {
            yew::virtual_dom::VNode::from(output)
        }
    }
}

pub fn calculate_lineinfo<P: Proof>(output: &mut HashMap<<P as Proof>::Reference, (usize, usize)>, prf: &<P as Proof>::Subproof, line: &mut usize, depth: &mut usize) {
    for prem in prf.premises() {
        output.insert(prem.clone(), (*line, *depth));
        *line += 1;
    }
    for lineref in prf.lines() {
        use frunk::Coproduct::{Inl, Inr};
        match lineref {
            Inl(r) => { output.insert(r, (*line, *depth)); *line += 1; },
            Inr(Inl(sr)) => { *depth += 1; calculate_lineinfo::<P>(output, &prf.lookup_subproof(sr).unwrap(), line, depth); *depth -= 1; },
            Inr(Inr(void)) => { match void {} },
        }
    }
}

pub fn initialize_inputs<P: Proof>(prf: &P) -> HashMap<<P as Proof>::Reference, String> {
    fn aux<P: Proof>(p: &<P as Proof>::Subproof, out: &mut HashMap<<P as Proof>::Reference, String>) {
        use frunk::Coproduct::{self, Inl, Inr};
        for line in p.premises().into_iter().map(Coproduct::inject).chain(p.lines().into_iter()) {
            match line {
                Inl(lr) => {
                    if let Some(e) = p.lookup_expr(lr.clone()) {
                        out.insert(lr.clone(), format!("{}", e));
                    }
                },
                Inr(Inl(sr)) => aux::<P>(&p.lookup_subproof(sr).unwrap(), out),
                Inr(Inr(void)) => match void {},
            }
        }
    }

    let mut out = HashMap::new();
    aux::<P>(prf.top_level_proof(), &mut out);
    out
}

fn may_remove_line<P: Proof>(prf: &P, proofref: &<P as Proof>::Reference) -> bool {
    use frunk::Coproduct::{Inl, Inr};
    let is_premise = match prf.lookup(proofref.clone()) {
        Some(Inl(_)) => true,
        Some(Inr(Inl(_))) => false,
        Some(Inr(Inr(void))) => match void {},
        None => panic!("prf.lookup failed in while processing a Delete"),
    };
    let parent = prf.parent_of_line(&frunk::Coproduct::inject(proofref.clone()));
    match parent.and_then(|x| prf.lookup_subproof(x)) {
        Some(sub) => (is_premise && sub.premises().len() > 1) || (!is_premise && sub.lines().len() > 1),
        None => (is_premise && prf.premises().len() > 1) || (!is_premise && prf.lines().len() > 1)
    }
}

impl Component for ProofWidget {
    type Message = ProofWidgetMsg;
    type Properties = ProofWidgetProps;
    fn create(props: Self::Properties, link: ComponentLink<Self>) -> Self {
        let mut prf;
        if props.blank {
            use expression_builders::var;
            prf = P::new();
            prf.add_premise(var(""));
            prf.add_step(Justification(var(""), RuleM::Reit, vec![], vec![]));
        } else {
            let data = include_bytes!("../../resolution_example.bram");
            let (prf2, _metadata) = crate::proofs::xml_interop::proof_from_xml::<P, _>(&data[..]).unwrap();
            prf = prf2;
        }

        let pud = ProofUiData::from_proof(&prf);
        let mut tmp = Self {
            link,
            prf,
            pud,
            selected_line: None,
            preblob: "".into(),
            props,
        };
        tmp.update(ProofWidgetMsg::Nop);
        tmp
    }
    fn update(&mut self, msg: Self::Message) -> ShouldRender {
        let mut ret = false;
        if self.props.verbose {
            self.preblob += &format!("{:?}\n", msg);
            ret = true;
        }
        use frunk::Coproduct::{Inl, Inr};
        match msg {
            ProofWidgetMsg::Nop => {},
            ProofWidgetMsg::LineChanged(r, input) => {
                self.pud.ref_to_input.insert(r.clone(), input.clone());
                if let Some(e) = crate::parser::parse(&input) {
                    match r {
                        Inl(_) => { self.prf.with_mut_premise(&r, |x| { *x = e }); },
                        Inr(Inl(_)) => { self.prf.with_mut_step(&r, |x| { x.0 = e }); },
                        Inr(Inr(void)) => match void {},
                    }
                }
                ret = true;
            },
            ProofWidgetMsg::LineAction(LineActionKind::Insert { what, after, relative_to }, orig_ref) => {
                use expression_builders::var;
                let to_select;
                let insertion_point = match relative_to {
                    LAKItem::Line => orig_ref,
                    LAKItem::Subproof => {
                        // TODO: need to refactor Proof::add_*_relative to take Coprod!(Reference, SubproofReference)
                        return ret;
                    },
                };
                match what {
                    LAKItem::Line => match insertion_point {
                        Inl(_) => { to_select = self.prf.add_premise_relative(var("__js_ui_blank_premise"), insertion_point, after); },
                        Inr(Inl(_)) => { to_select = self.prf.add_step_relative(Justification(var("__js_ui_blank_step"), RuleM::Reit, vec![], vec![]), insertion_point, after); },
                        Inr(Inr(void)) => match void {},
                    },
                    LAKItem::Subproof => {
                        let sr = self.prf.add_subproof_relative(insertion_point, after);
                        to_select = self.prf.with_mut_subproof(&sr, |sub| {
                            let to_select = sub.add_premise(var("__js_ui_blank_premise"));
                            sub.add_step(Justification(var("__js_ui_blank_step"), RuleM::Reit, vec![], vec![]));
                            to_select
                        }).unwrap();
                    },
                }
                self.selected_line = Some(to_select);
                self.preblob += &format!("{:?}\n", self.prf.premises());
                ret = true;
            },
            ProofWidgetMsg::LineAction(LineActionKind::Delete { what }, proofref) => {
                let parent = self.prf.parent_of_line(&frunk::Coproduct::inject(proofref.clone()));
                match what {
                    LAKItem::Line => {
                        fn remove_line_if_allowed<P: Proof, Q: Proof<Reference=<P as Proof>::Reference>>(prf: &mut Q, pud: &mut ProofUiData<P>, proofref: <Q as Proof>::Reference) {
                            pud.ref_to_line_depth.remove(&proofref);
                            pud.ref_to_input.remove(&proofref);
                            if may_remove_line(prf, &proofref) {
                                prf.remove_line(proofref);
                            }
                        }
                        match parent {
                            Some(sr) => { let pud = &mut self.pud; self.prf.with_mut_subproof(&sr, |sub| { remove_line_if_allowed(sub, pud, proofref); }); },
                            None => { remove_line_if_allowed(&mut self.prf, &mut self.pud, proofref); },
                        }
                    },
                    LAKItem::Subproof => {
                        // TODO: recursively clean out the ProofUiData entries for lines inside a subproof before deletion
                        match parent {
                            Some(sr) => { self.prf.remove_subproof(sr); },
                            None => {}, // shouldn't delete the root subproof
                        }
                    },
                }
                ret = true;
            },
            ProofWidgetMsg::LineAction(LineActionKind::SetRule { rule }, proofref) => {
                self.prf.with_mut_step(&proofref, |j| { j.1 = rule });
                self.selected_line = Some(proofref);
                ret = true;
            },
            ProofWidgetMsg::LineAction(LineActionKind::Select, proofref) => {
                self.selected_line = Some(proofref);
                ret = true;
            },
            ProofWidgetMsg::LineAction(LineActionKind::SetDependency { to, dep }, proofref) => {
                self.prf.with_mut_step(&proofref, |j| {
                    fn toggle_dep_or_sdep<T: Ord>(dep: T, deps: &mut Vec<T>, to: bool) {
                        let mut dep_set: BTreeSet<T> = mem::replace(deps, vec![]).into_iter().collect();
                        if to {
                            dep_set.insert(dep);
                        } else {
                            dep_set.remove(&dep);
                        }
                        deps.extend(dep_set);
                    }
                    match dep {
                        Inl(lr) => toggle_dep_or_sdep(lr, &mut j.2, to),
                        Inr(Inl(sr)) => toggle_dep_or_sdep(sr, &mut j.3, to),
                        Inr(Inr(void)) => match void {},
                    }
                });
                ret = true;
            }
        }
        if ret {
            calculate_lineinfo::<P>(&mut self.pud.ref_to_line_depth, self.prf.top_level_proof(), &mut 1, &mut 0);
        }
        ret
    }
    fn view(&self) -> Html {
        let interactive_proof = self.render_proof(self.prf.top_level_proof(), None, &mut 1, &mut 0);
        html! {
            <div>
                { interactive_proof }
                <div style="display: none">
                    <hr />
                    <pre> { format!("{}\n{:#?}", self.prf, self.prf) } </pre>
                    <hr />
                    <pre> { self.preblob.clone() } </pre>
                </div>
            </div>
        }
    }
}

pub struct TabbedContainer {
    link: ComponentLink<Self>,
    tabs: Vec<(String, Html)>,
    current_tab: usize,
}

#[derive(Clone,Properties)]
pub struct TabbedContainerProps {
    tab_ids: Vec<String>,
    children: Children,
}

impl Component for TabbedContainer {
    type Message = usize;
    type Properties = TabbedContainerProps;

    fn create(props: Self::Properties, link: ComponentLink<Self>) -> Self {
        let tabs: Vec<(String, Html)> = props.tab_ids.into_iter().zip(props.children.to_vec().into_iter()).collect();
        Self { link, tabs, current_tab: 0 }
    }

    fn update(&mut self, msg: Self::Message) -> ShouldRender {
        self.current_tab = msg;
        true
    }

    fn view(&self) -> Html {
        let mut tab_links = yew::virtual_dom::VList::new();
        let mut out = yew::virtual_dom::VList::new();
        for (i, (name, data)) in self.tabs.iter().enumerate() {
            tab_links.add_child(html! { <input type="button" onclick=self.link.callback(move |_| i) value=name /> });
            if i == self.current_tab {
                out.add_child(html! { <div> { data.clone() } </div> });
            } else {
                out.add_child(html! { <div style="display:none"> { data.clone() } </div> });
            }
        }

        html! {
            <div>
                <div> { tab_links }</div>
                { out }
            </div>
        }
    }
}

pub struct App {
    link: ComponentLink<Self>,
    last_good_parse: String,
    current_expr: Option<Expr>,
}

pub enum Msg {
    ExprChanged(String, Option<Expr>),
}

impl Component for App {
    type Message = Msg;
    type Properties = ();

    fn create(_: Self::Properties, link: ComponentLink<Self>) -> Self {
        Self { link, last_good_parse: "".into(), current_expr: None }
    }

    fn update(&mut self, msg: Self::Message) -> ShouldRender {
        match msg {
            Msg::ExprChanged(last_good_parse, current_expr) => {
                self.last_good_parse = last_good_parse;
                self.current_expr = current_expr;
                true
            },
        }
    }

    fn view(&self) -> Html {
        let exprwidget = html! {
            <div>
                <p>{ "Enter Expression:" }</p>
                <ExprEntry initial_contents="forall A, ((exists B, A -> B) & C & f(x, y | z)) <-> Q <-> R" onchange=self.link.callback(|(x, y)| Msg::ExprChanged(x, y)) />
            </div>
        };
        let tabview = html! {
            <TabbedContainer tab_ids=vec!["Resolution example".into(), "Untitled proof".into(), "Parser demo".into()]>
                <ProofWidget verbose=true blank=false />
                <ProofWidget verbose=true blank=true />
                { exprwidget }
            </TabbedContainer>
        };
        html! {
            <div>
                { tabview }
            </div>
        }
    }
}